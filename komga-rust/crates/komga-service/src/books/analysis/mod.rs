//! Book analysis: Kotlin `BookLifecycle.analyzeAndPersist`, `BookController.bookAnalyze` and
//! `SeriesController.seriesAnalyze`.

mod book_analyzer;
mod epub_analyzer;
mod failure;
mod mobi_analyzer;
pub(crate) mod pdf_extractor;
mod rar_extractor;
mod zip_extractor;

pub(crate) use book_analyzer::analyze;

use std::io;

use komga_db::{
    BookDao, Database, DbError, LibraryDao, MediaDao, ReadProgressDao, SearchIndexPendingDao,
};
use komga_scheduler::{Task, TaskQueue};
use komga_types::{
    Book, BookId, BookMetadataPatchCapability, BookRef, Dimension, DomainEvent, FileUrl, KomgaUser,
    LibraryId, MediaStatus, SearchEntity, SeriesId,
};
use thiserror::Error;
use tokio::task::{self, JoinError};
use tracing::{info, warn};

use crate::catalog::search_index::SearchIndexNotifier;
use crate::events::EventBus;

/// Kotlin `MediaContainerEntry`.
struct ContainerEntry {
    pub name: String,
    /// `None` when the member could not be read.
    pub media_type: Option<&'static str>,
    pub dimension: Option<Dimension>,
    pub file_size: Option<i64>,
}

#[derive(Debug, Error)]
pub enum BookAnalysisError {
    #[error("Book not found")]
    NotFound,
    #[error("Forbidden")]
    Forbidden,
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error("Analysis worker failed: {0}")]
    Worker(#[from] JoinError),
    #[error("Book URL {0} is not a local file")]
    InvalidUrl(FileUrl),
    #[error("Book {0} has no media")]
    MissingMedia(BookId),
    /// A ZSTD or XZ member, whose missing codec class fails Kotlin's whole analysis.
    #[error("compression method {0} needs a codec Komga does not ship")]
    MissingCodec(u16),
    /// A read failure the file may not have next time, which must not be stored as a broken book.
    #[error("Could not read the book file: {0}")]
    Io(io::Error),
}

#[derive(Clone)]
pub struct BookAnalysisService {
    db: Database,
    tasks: TaskQueue,
    /// `komga.epub-divina-letter-count-threshold`.
    epub_letter_count_threshold: i32,
    search: SearchIndexNotifier,
    events: EventBus,
}

impl BookAnalysisService {
    #[must_use]
    pub fn new(
        db: Database,
        tasks: TaskQueue,
        epub_letter_count_threshold: i32,
        search: SearchIndexNotifier,
        events: EventBus,
    ) -> Self {
        Self {
            db,
            tasks,
            epub_letter_count_threshold,
            search,
            events,
        }
    }

    /// Queues `AnalyzeBook` ahead of default tasks; soft-deleted books are analyzed too.
    pub async fn submit(&self, user: &KomgaUser, id: &BookId) -> Result<(), BookAnalysisError> {
        if !user.is_admin() {
            return Err(BookAnalysisError::Forbidden);
        }
        let mut tx = self.db.begin().await?;
        let book = BookDao::find_by_id(&mut tx, id).await?;
        tx.rollback().await?;
        let book = book.ok_or(BookAnalysisError::NotFound)?;
        self.tasks
            .submit(&Task::analyze_book(book.id, book.series_id).high())
            .await?;
        Ok(())
    }

    /// Queues `AnalyzeBook` for every book of the series, soft-deleted ones included. Kotlin does
    /// not check that the series exists: an unknown one queues nothing.
    pub async fn submit_series(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
    ) -> Result<(), BookAnalysisError> {
        if !user.is_admin() {
            return Err(BookAnalysisError::Forbidden);
        }
        let mut tx = self.db.begin().await?;
        let books = BookDao::find_all_by_series(&mut tx, id).await?;
        tx.rollback().await?;
        self.submit_all(books).await
    }

    /// As [`Self::submit_series`], for every book of the library.
    pub async fn submit_library(
        &self,
        user: &KomgaUser,
        id: &LibraryId,
    ) -> Result<(), BookAnalysisError> {
        if !user.is_admin() {
            return Err(BookAnalysisError::Forbidden);
        }
        let mut tx = self.db.begin().await?;
        let books = BookDao::find_all_by_library(&mut tx, id).await?;
        tx.rollback().await?;
        self.submit_all(books).await
    }

    async fn submit_all(&self, books: Vec<Book>) -> Result<(), BookAnalysisError> {
        let tasks: Vec<_> = books
            .into_iter()
            .map(|book| Task::analyze_book(book.id, book.series_id).high())
            .collect();
        self.tasks.submit_all(&tasks).await?;
        Ok(())
    }

    /// Analyzes the book file and stores its media in one transaction. A ready book then gets its
    /// thumbnail and metadata refreshed at `priority + 1`.
    ///
    /// Results of a file version the database does not describe are dropped: when the file is not
    /// the recorded version or changes during analysis, a deep scan is queued to record it (an
    /// in-place rewrite leaves the directory time that ordinary scans check); when a scan
    /// recorded one meanwhile, that scan already queued a new analysis.
    pub(crate) async fn analyze_and_persist(
        &self,
        id: &BookId,
        priority: i32,
    ) -> Result<(), BookAnalysisError> {
        let mut tx = self.db.begin().await?;
        let Some(book) = BookDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            warn!(book = %id, "Cannot analyze book: Book does not exist");
            return Ok(());
        };
        let library = LibraryDao::find_by_id(&mut tx, &book.library_id)
            .await?
            .expect("A book's library exists (foreign key)");
        tx.rollback().await?;
        info!(book = %book.id, url = %book.url, "Analyze and persist book");

        let path = book
            .url
            .to_path()
            .ok_or_else(|| BookAnalysisError::InvalidUrl(book.url.clone()))?;
        let analyze_dimensions = library.analyze_dimensions;
        let threshold = self.epub_letter_count_threshold;
        let recorded = book.clone();
        let analyzed = task::spawn_blocking(move || {
            book_analyzer::analyze_recorded(&path, &recorded, analyze_dimensions, threshold)
        })
        .await??;
        let Some(media) = analyzed else {
            warn!(book = %book.id, "Book file is not the recorded version, scanning the library");
            self.tasks
                .submit(&Task::scan_library(library.id, true))
                .await?;
            return Ok(());
        };

        let mut tx = self.db.begin_immediate().await?;
        let current = BookDao::find_by_id(&mut tx, id).await?;
        let same_version = current.is_some_and(|current| {
            (current.file_last_modified, current.file_size)
                == (book.file_last_modified, book.file_size)
        });
        if !same_version {
            tx.rollback().await?;
            warn!(book = %id, "Book changed during analysis, dropping the result");
            return Ok(());
        }
        let previous = MediaDao::find_by_id(&mut tx, id)
            .await?
            .ok_or_else(|| BookAnalysisError::MissingMedia(id.clone()))?;
        if previous.status == MediaStatus::Outdated && previous.page_count != media.page_count {
            info!(book = %id, "Number of pages differ, adjust read progress for book");
            ReadProgressDao::reset_pages(&mut tx, id, media.page_count).await?;
        }
        MediaDao::update(&mut tx, id, &media).await?;
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::Book, &[id.as_str()]).await?;
        tx.record(DomainEvent::BookUpdated(BookRef::from(&book)));
        self.events.commit(tx).await?;
        self.search.notify();

        if media.status == MediaStatus::Ready {
            self.tasks
                .submit_all(&[
                    Task::generate_book_thumbnail(book.id.clone()).with_priority(priority + 1),
                    Task::refresh_book_metadata(
                        book.id,
                        book.series_id,
                        BookMetadataPatchCapability::ALL.to_vec(),
                    )
                    .with_priority(priority + 1),
                ])
                .await?;
        }
        Ok(())
    }
}
