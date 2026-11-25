//! Kotlin `BookLifecycle.markReadProgress`, `markReadProgressCompleted`, `deleteReadProgress`,
//! and `markProgression`, and their series counterparts.

mod progression;
mod series;
mod tachiyomi;

use jiff::Timestamp;
use komga_archive::epub::MalformedHref;
use komga_db::{BookDao, Database, DbError, DbTransaction, MediaDao, ReadProgressDao};
use komga_types::{
    Book, BookId, BookProgressRef, DomainEvent, KomgaUser, Media, MediaProfile, ReadProgress,
};
use thiserror::Error;
use tracing::error;

use crate::events::EventBus;
use crate::support::access_policy;

pub use tachiyomi::{ReadListTachiyomiProgress, SeriesTachiyomiProgress};

#[derive(Debug, Error)]
pub enum ReadProgressError {
    #[error("Book not found")]
    NotFound,
    /// `ContentRestrictionChecker` denied the book.
    #[error("Content restricted")]
    Restricted,
    /// The page is `None` for a progression without a position.
    #[error(
        "Page argument ({}) must be within 1 and book page count ({page_count})",
        page.map_or_else(|| "null".to_owned(), |page| page.to_string())
    )]
    PageOutOfRange { page: Option<i32>, page_count: i32 },
    #[error("epub book is not Divina compatible")]
    NotDivinaCompatible,
    #[error("Epub extension not found")]
    EpubExtensionNotFound,
    /// The stored positions of a DiViNa EPUB are fewer than its pages.
    #[error("Book {0} has no position for page {1}")]
    MissingPosition(BookId, i32),
    /// The stored progress was read at or after the reported progression.
    #[error("Progression is older than existing")]
    OutdatedProgression,
    #[error("Media has no profile")]
    NoProfile,
    #[error(transparent)]
    MalformedHref(#[from] MalformedHref),
    #[error("Resource does not exist in book: {0}")]
    UnknownResource(String),
    #[error("location.progression is required")]
    ProgressionRequired,
    /// No position of the resource matches or surrounds the progression.
    #[error("Invalid progression")]
    InvalidProgression,
    #[error("Book {0} has no media")]
    MissingMedia(BookId),
    #[error("Database error: {0}")]
    Db(#[from] DbError),
}

#[derive(Clone)]
pub struct ReadProgressService {
    db: Database,
    events: EventBus,
}

impl ReadProgressService {
    #[must_use]
    pub fn new(db: Database, events: EventBus) -> Self {
        Self { db, events }
    }

    /// `markReadProgress`: the user is at `page`, which completes the book when it is the last.
    /// A DiViNa EPUB also keeps the position of the page as its locator.
    pub async fn mark_page(
        &self,
        user: &KomgaUser,
        id: &BookId,
        page: i32,
    ) -> Result<(), ReadProgressError> {
        let mut tx = self.db.begin_immediate().await?;
        let book = visible_book(&mut tx, user, id).await?;
        let media = media(&mut tx, &book.id).await?;
        if !(1..=media.page_count).contains(&page) {
            return Err(ReadProgressError::PageOutOfRange {
                page: Some(page),
                page_count: media.page_count,
            });
        }
        let locator = if MediaProfile::from_media_type(media.media_type.as_deref())
            == Some(MediaProfile::Epub)
        {
            if !media.epub_divina_compatible {
                return Err(ReadProgressError::NotDivinaCompatible);
            }
            let Some(extension) = MediaDao::find_extension(&mut tx, &book.id).await? else {
                error!(book = %book.id, "Epub extension not found, the book should be re-analyzed");
                return Err(ReadProgressError::EpubExtensionNotFound);
            };
            let index = usize::try_from(page - 1).expect("The page is at least 1");
            let position = extension.positions.into_iter().nth(index);
            Some(position.ok_or(ReadProgressError::MissingPosition(id.clone(), page))?)
        } else {
            None
        };
        let progress = ReadProgress {
            completed: page == media.page_count,
            locator,
            ..new_progress(&book, user, page)
        };
        self.save(tx, &progress).await
    }

    /// `markReadProgressCompleted`: the user has read the book to its last page.
    pub async fn mark_completed(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<(), ReadProgressError> {
        let mut tx = self.db.begin_immediate().await?;
        let book = visible_book(&mut tx, user, id).await?;
        let media = media(&mut tx, &book.id).await?;
        let progress = ReadProgress {
            completed: true,
            ..new_progress(&book, user, media.page_count)
        };
        self.save(tx, &progress).await
    }

    /// `deleteReadProgress`: the book is unread again for the user.
    pub async fn delete(&self, user: &KomgaUser, id: &BookId) -> Result<(), ReadProgressError> {
        let mut tx = self.db.begin_immediate().await?;
        let book = visible_book(&mut tx, user, id).await?;
        if ReadProgressDao::delete(&mut tx, &book.id, &user.id).await? {
            record_books(
                &mut tx,
                vec![book.id],
                user,
                DomainEvent::ReadProgressDeleted,
            );
        }
        self.events.commit(tx).await?;
        Ok(())
    }

    /// Saves `progress` and commits.
    async fn save(
        &self,
        mut tx: DbTransaction,
        progress: &ReadProgress,
    ) -> Result<(), ReadProgressError> {
        ReadProgressDao::save(&mut tx, progress).await?;
        tx.record(DomainEvent::ReadProgressChanged(BookProgressRef {
            book_id: progress.book_id.clone(),
            user_id: progress.user_id.clone(),
        }));
        self.events.commit(tx).await?;
        Ok(())
    }
}

/// Records `event` for `user`'s progress on each of `book_ids`.
fn record_books(
    tx: &mut DbTransaction,
    book_ids: Vec<BookId>,
    user: &KomgaUser,
    event: fn(BookProgressRef) -> DomainEvent,
) {
    for book_id in book_ids {
        tx.record(event(BookProgressRef {
            book_id,
            user_id: user.id.clone(),
        }));
    }
}

/// A progress read now on no particular device.
fn new_progress(book: &Book, user: &KomgaUser, page: i32) -> ReadProgress {
    ReadProgress {
        book_id: book.id.clone(),
        user_id: user.id.clone(),
        page,
        completed: false,
        read_date: Timestamp::now(),
        device_id: String::new(),
        device_name: String::new(),
        locator: None,
    }
}

async fn visible_book(
    transaction: &mut DbTransaction,
    user: &KomgaUser,
    id: &BookId,
) -> Result<Book, ReadProgressError> {
    let book = BookDao::find_by_id(transaction, id)
        .await?
        .ok_or(ReadProgressError::NotFound)?;
    if access_policy::can_access_book(transaction, user, &book).await? {
        Ok(book)
    } else {
        Err(ReadProgressError::Restricted)
    }
}

async fn media(transaction: &mut DbTransaction, id: &BookId) -> Result<Media, ReadProgressError> {
    MediaDao::find_by_id(transaction, id)
        .await?
        .ok_or_else(|| ReadProgressError::MissingMedia(id.clone()))
}
