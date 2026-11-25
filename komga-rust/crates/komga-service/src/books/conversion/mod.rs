//! Kotlin `BookConverter`: a RAR book becomes a CBZ of the same members, and a book file gets the
//! extension of its media type, keeping the book's ID. The new file is published beside the old
//! one, which is removed only once the database describes the new file, so a failure or a crash
//! leaves one readable book file.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use komga_archive::rar::RarError;
use komga_archive::zip::ZipError;
use komga_db::{
    BookDao, Database, DbError, HistoricalEventDao, LibraryDao, MediaDao, PageHashDao,
    ReadProgressDao, SearchIndexPendingDao, SeriesDao,
};
use komga_scheduler::{Task, TaskQueue};
use komga_types::{
    Book, BookId, BookPageNumbered, BookRef, DomainEvent, HistoricalEvent, Library, LibraryId,
    Media, MediaStatus, SearchEntity,
};
use thiserror::Error;
use tokio::task::{self, JoinError};
use tracing::{info, warn};

use self::cbz::{member_names, restore_hashes, verify, write_cbz};
use crate::books::analysis;
use crate::catalog::search_index::SearchIndexNotifier;
use crate::events::EventBus;
use crate::library::file_operation::{
    FileOperationError, FileOperations, PendingFile, SourceAfter,
};
use crate::library::scanning::scan_file;
use crate::support::file_time;

mod cbz;
mod extension_repair;
mod page_removal;

pub(crate) use page_removal::removal_tasks;
#[cfg(test)]
mod tests;

/// Kotlin `BookConverter.convertibleTypes`.
pub(crate) const CONVERTIBLE_TYPES: [&str; 2] = [
    "application/x-rar-compressed; version=4",
    "application/x-rar-compressed; version=5",
];
const ZIP: &str = "application/zip";
/// Upper bound per in-memory skip/failure set so long-running servers cannot grow unboundedly.
const MAX_TRACKED_BOOKS: usize = 10_000;

/// FIFO-bounded set of book IDs remembered until restart.
#[derive(Default)]
struct BoundedBookSet {
    ids: HashSet<BookId>,
    order: VecDeque<BookId>,
}

impl BoundedBookSet {
    fn contains(&self, id: &BookId) -> bool {
        self.ids.contains(id)
    }

    fn insert(&mut self, id: BookId) {
        if !self.ids.insert(id.clone()) {
            return;
        }
        self.order.push_back(id);
        if self.order.len() > MAX_TRACKED_BOOKS
            && let Some(oldest) = self.order.pop_front()
        {
            self.ids.remove(&oldest);
        }
    }
}

#[derive(Debug, Error)]
pub enum ConversionError {
    #[error("File not found: {}", .0.display())]
    FileNotFound(PathBuf),
    #[error("{0} cannot be converted. Must be one of {1:?}")]
    MediaUnsupported(String, &'static [&'static str]),
    #[error("{0} cannot be repaired. Must be a known book type")]
    NotRepairable(String),
    #[error("Book media is not ready")]
    MediaNotReady,
    #[error("{0}, aborting")]
    Unverified(&'static str),
    #[error(transparent)]
    FileOperation(#[from] FileOperationError),
    #[error("Cannot read the RAR file: {0}")]
    Rar(#[from] RarError),
    #[error("Cannot read the ZIP file: {0}")]
    Zip(#[from] ZipError),
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error("Filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("Filesystem worker failed: {0}")]
    Worker(#[from] JoinError),
}

#[derive(Clone)]
pub struct BookConverter {
    db: Database,
    tasks: TaskQueue,
    files: FileOperations,
    epub_letter_count_threshold: i32,
    search: SearchIndexNotifier,
    events: EventBus,
    /// Books whose CBZ failed verification; as in Kotlin, until the server restarts (bounded).
    failed: Arc<Mutex<BoundedBookSet>>,
    /// Books whose page removal failed verification; as in Kotlin, until the server restarts (bounded).
    removal_failed: Arc<Mutex<BoundedBookSet>>,
    /// Books whose extension needs no repair; as in Kotlin, until the server restarts (bounded).
    skipped: Arc<Mutex<BoundedBookSet>>,
}

/// A book with its media, as read before its file changes.
struct Original {
    book: Book,
    media: Media,
    analyze_dimensions: bool,
}

impl BookConverter {
    #[must_use]
    pub fn new(
        db: Database,
        tasks: TaskQueue,
        epub_letter_count_threshold: i32,
        search: SearchIndexNotifier,
        events: EventBus,
    ) -> Self {
        Self {
            files: FileOperations::new(db.clone()),
            db,
            tasks,
            epub_letter_count_threshold,
            search,
            events,
            failed: Arc::default(),
            removal_failed: Arc::default(),
            skipped: Arc::default(),
        }
    }

    /// Kotlin `FindBooksToConvert`: queues a conversion of each RAR book of a library that
    /// converts them.
    pub(crate) async fn find_books_to_convert(
        &self,
        library_id: &LibraryId,
        priority: i32,
    ) -> Result<(), ConversionError> {
        let mut tx = self.db.begin().await?;
        let Some(library) = LibraryDao::find_by_id(&mut tx, library_id).await? else {
            tx.rollback().await?;
            warn!(library = %library_id, "Cannot find books to convert: Library does not exist");
            return Ok(());
        };
        if !library.convert_to_cbz {
            tx.rollback().await?;
            info!(library = %library_id, "CBZ conversion is not enabled, skipping");
            return Ok(());
        }
        let books =
            BookDao::find_all_by_library_and_media_types(&mut tx, library_id, &CONVERTIBLE_TYPES)
                .await?;
        tx.rollback().await?;
        info!(library = %library_id, count = books.len(), "Found books to convert");
        let tasks: Vec<Task> = books
            .into_iter()
            .map(|book| Task::convert_book(book.id, book.series_id).with_priority(priority + 1))
            .collect();
        self.tasks.submit_all(&tasks).await?;
        Ok(())
    }

    pub(crate) async fn convert_to_cbz(&self, id: &BookId) -> Result<(), ConversionError> {
        let Some(convertible) = self.convertible(id).await? else {
            return Ok(());
        };
        let path = book_path(&convertible.book);
        let target = path.with_extension("cbz");
        info!(target = %target.display(), "Copying archive content");
        let mut file = self
            .files
            .begin(&path, &target, None, SourceAfter::Remove)
            .await?;
        let names = member_names(&convertible.media);
        let write = move |source: &Path, target: &Path| write_cbz(source, target, &names);
        let result = self
            .write_and_record(&mut file, &convertible, &convertible.media, write, &[], &[])
            .await
            .map(drop);
        self.files.finish(file).await?;
        if let Err(ConversionError::Unverified(_)) = &result {
            self.failed.lock().expect("Not poisoned").insert(id.clone());
        }
        result
    }

    /// The book and its media when the conversion should go ahead; `None` when it is skipped.
    async fn convertible(&self, id: &BookId) -> Result<Option<Original>, ConversionError> {
        let enabled = |library: &Library| library.convert_to_cbz;
        let Some((book, library, media)) = self.changeable(id, enabled, &self.failed).await? else {
            return Ok(None);
        };
        let media_type = media.media_type.clone().unwrap_or_default();
        if !CONVERTIBLE_TYPES.contains(&media_type.as_str()) {
            return Err(ConversionError::MediaUnsupported(
                media_type,
                &CONVERTIBLE_TYPES,
            ));
        }
        if media.status != MediaStatus::Ready {
            return Err(ConversionError::MediaNotReady);
        }
        Ok(Some(Original {
            book,
            media,
            analyze_dimensions: library.analyze_dimensions,
        }))
    }

    /// The book with its library and media; `None`, logged, when the book is gone, the library
    /// option is off, `skipped` holds the book, or its file changed since it was recorded.
    async fn changeable(
        &self,
        id: &BookId,
        enabled: impl Fn(&Library) -> bool,
        skipped: &Mutex<BoundedBookSet>,
    ) -> Result<Option<(Book, Library, Media)>, ConversionError> {
        let mut tx = self.db.begin().await?;
        let Some(book) = BookDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            warn!(book = %id, "Book does not exist, skipping");
            return Ok(None);
        };
        let library = LibraryDao::find_by_id(&mut tx, &book.library_id)
            .await?
            .expect("A book's library exists (foreign key)");
        let media = MediaDao::find_by_id(&mut tx, id)
            .await?
            .expect("Books have media");
        tx.rollback().await?;
        if !enabled(&library) {
            info!(book = %id, "The library option is disabled, skipping");
            return Ok(None);
        }
        if skipped.lock().expect("Not poisoned").contains(id) {
            info!(book = %id, "Book was skipped or failed before, skipping");
            return Ok(None);
        }
        let path = book_path(&book);
        let metadata_path = path.clone();
        let metadata = match task::spawn_blocking(move || fs::metadata(&metadata_path)).await? {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(ConversionError::FileNotFound(path));
            }
            metadata => metadata?,
        };
        if !file_time::is_recorded(&metadata, &book) {
            info!(book = %id, "Book has changed on disk, skipping");
            return Ok(None);
        }
        Ok(Some((book, library, media)))
    }

    /// Writes the new CBZ with `write(source, temporary)`, verifies it holds the `expected`
    /// pages and files, publishes it and records it with the `deleted` pages; on failure the
    /// caller's `finish` drops it. Whether the book was recorded.
    async fn write_and_record(
        &self,
        file: &mut PendingFile,
        original: &Original,
        expected: &Media,
        write: impl FnOnce(&Path, &Path) -> Result<(), ConversionError> + Send + 'static,
        deleted: &[BookPageNumbered],
        follow_ups: &[Task],
    ) -> Result<bool, ConversionError> {
        let (source, temporary) = (file.source().to_owned(), file.temporary().to_owned());
        let threshold = self.epub_letter_count_threshold;
        let analyze_dimensions = original.analyze_dimensions;
        let converted = task::spawn_blocking(move || {
            write(&source, &temporary)?;
            analysis::analyze(&temporary, analyze_dimensions, threshold)
                .map_err(|_| ConversionError::Unverified("Created file could not be analyzed"))
        })
        .await??;
        verify(expected, &converted)?;
        self.files.publish(file).await?;
        let target = file.target().to_owned();
        let library_id = original.book.library_id.clone();
        let scanned = task::spawn_blocking(move || scan_file(&target, &library_id))
            .await?
            .ok_or_else(|| ConversionError::FileNotFound(file.target().to_owned()))?;
        let book = Book {
            name: scanned.name,
            url: scanned.url,
            file_last_modified: scanned.file_last_modified,
            file_size: scanned.file_size,
            file_hash: String::new(),
            file_hash_koreader: String::new(),
            ..original.book.clone()
        };
        let media = Media {
            pages: restore_hashes(converted.pages, &original.media.pages),
            ..converted
        };
        self.record(
            file,
            &original.book,
            &book,
            Some(&media),
            deleted,
            follow_ups,
        )
        .await
    }

    /// Points the book, and its `media` if changed, at the published file in one transaction,
    /// counting the `deleted` pages against their known hashes and queuing `follow_ups`, unless a
    /// scan changed the book meanwhile. Whether the book was recorded.
    async fn record(
        &self,
        file: &PendingFile,
        previous: &Book,
        book: &Book,
        media: Option<&Media>,
        deleted: &[BookPageNumbered],
        follow_ups: &[Task],
    ) -> Result<bool, ConversionError> {
        let mut tx = self.db.begin_immediate().await?;
        let current = BookDao::find_by_id(&mut tx, &book.id).await?;
        if current.as_ref() != Some(previous) {
            tx.rollback().await?;
            warn!(book = %book.id, "Book changed during the file change, dropping the result");
            return Ok(false);
        }
        BookDao::update(&mut tx, book).await?;
        if let Some(media) = media {
            let page_count = MediaDao::find_by_id(&mut tx, &book.id)
                .await?
                .map(|m| m.page_count);
            // The scan sees the new file as already analyzed, so progress is adjusted here.
            if page_count != Some(media.page_count) {
                ReadProgressDao::reset_pages(&mut tx, &book.id, media.page_count).await?;
            }
            MediaDao::update(&mut tx, &book.id, media).await?;
        }
        // A oneshot series is its book file.
        if book.oneshot {
            SeriesDao::update_file(&mut tx, &book.series_id, &book.url, book.file_last_modified)
                .await?;
        }
        for page in deleted {
            PageHashDao::count_deletion(&mut tx, &page.file_hash).await?;
            let event = HistoricalEvent::duplicate_page_deleted(previous, page);
            HistoricalEventDao::insert(&mut tx, &event).await?;
        }
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::Book, &[book.id.as_str()]).await?;
        self.files.commit(&mut tx, file).await?;
        TaskQueue::stage(&mut tx, follow_ups).await?;
        tx.record(DomainEvent::BookUpdated(BookRef::from(book)));
        self.events.commit(tx).await?;
        self.search.notify();
        self.tasks.hand_over(&self.db).await;
        info!(book = %book.id, url = %book.url, "Book file changed");
        Ok(true)
    }
}

fn book_path(book: &Book) -> PathBuf {
    book.url.to_path().expect("Books have file URLs")
}
