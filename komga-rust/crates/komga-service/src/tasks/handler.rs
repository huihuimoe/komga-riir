//! Kotlin `TaskHandler`: runs each claimed task through its business use case.

use komga_db::{BookHash, Database};
use komga_scheduler::{Task, TaskExecutor, TaskFailure, TaskKind, TaskQueue};
use komga_types::LibraryId;
use tokio_util::sync::CancellationToken;

use super::failures::{
    analysis_failure, artwork_failure, conversion_failure, deletion_failure, failure,
    hashing_failure, import_failure, index_result, metadata_failure,
};

use crate::artwork::{ArtworkError, ArtworkService};
use crate::books::analysis::BookAnalysisService;
use crate::books::conversion::{BookConverter, ConversionError};
use crate::books::hashing::{HashingError, HashingService};
use crate::books::importing::BookImporter;
use crate::catalog::search_index::{SearchIndexNotifier, SearchIndexUpdater};
use crate::events::EventBus;
use crate::library::file_deletion::FileDeletion;
use crate::library::scanning::LibraryContentService;
use crate::metadata::{MetadataError, MetadataService};
use crate::reading::ReadingService;

/// Configuration for background task handlers.
#[derive(Debug, Clone, Copy)]
pub struct TaskHandlerConfig {
    pub epub_letter_count_threshold: i32,
    pub page_hashing: i32,
}

pub struct TaskHandler {
    library_content: LibraryContentService,
    book_analysis: BookAnalysisService,
    artwork: ArtworkService,
    hashing: HashingService,
    metadata: MetadataService,
    importer: BookImporter,
    converter: BookConverter,
    deletion: FileDeletion,
    search_index: SearchIndexUpdater,
}

impl TaskHandler {
    #[must_use]
    pub fn new(
        db: Database,
        tasks: TaskQueue,
        config: TaskHandlerConfig,
        reading: ReadingService,
        search: SearchIndexNotifier,
        events: EventBus,
        search_index: SearchIndexUpdater,
    ) -> Self {
        Self {
            library_content: LibraryContentService::new(
                db.clone(),
                tasks.clone(),
                search.clone(),
                events.clone(),
            ),
            book_analysis: BookAnalysisService::new(
                db.clone(),
                tasks.clone(),
                config.epub_letter_count_threshold,
                search.clone(),
                events.clone(),
            ),
            artwork: ArtworkService::new(db.clone(), tasks.clone(), events.clone()),
            hashing: HashingService::new(db.clone(), tasks.clone(), config.page_hashing),
            converter: BookConverter::new(
                db.clone(),
                tasks.clone(),
                config.epub_letter_count_threshold,
                search.clone(),
                events.clone(),
            ),
            importer: BookImporter::new(db.clone(), tasks.clone(), search.clone(), events.clone()),
            deletion: FileDeletion::new(db.clone(), tasks.clone(), search.clone(), events.clone()),
            metadata: MetadataService::new(db, tasks, reading, search, events),
            search_index,
        }
    }

    /// Scans the library, then queues the follow-ups of a library that is still there.
    async fn scan_library(
        &self,
        library_id: &LibraryId,
        scan_deep: bool,
        stop: &CancellationToken,
    ) -> Result<(), TaskFailure> {
        let scanned = self
            .library_content
            .scan_root_folder(library_id, scan_deep, stop)
            .await
            .map_err(failure)?;
        match scanned {
            Some(library) => self
                .library_content
                .submit_scan_follow_ups(&library)
                .await
                .map_err(failure),
            None => Ok(()),
        }
    }
}

impl TaskHandler {
    async fn execute_artwork(&self, task: Task) -> Result<(), ArtworkError> {
        let Task { kind, priority } = task;
        match kind {
            TaskKind::GenerateBookThumbnail { book_id } => {
                match self.artwork.generate_and_persist(&book_id).await {
                    // A book without a usable cover is a normal outcome, not a task failure.
                    Err(error @ ArtworkError::NoCover(_)) => {
                        tracing::warn!(%error, "No thumbnail or poster found");
                        Ok(())
                    }
                    result => result,
                }
            }
            TaskKind::RefreshBookLocalArtwork { book_id } => {
                self.artwork.refresh_book_local_artwork(&book_id).await
            }
            TaskKind::RefreshSeriesLocalArtwork { series_id } => {
                self.artwork.refresh_series_local_artwork(&series_id).await
            }
            TaskKind::FindBookThumbnailsToRegenerate {
                for_bigger_result_only,
            } => {
                self.artwork
                    .find_thumbnails_to_regenerate(for_bigger_result_only, priority)
                    .await
            }
            other => unreachable!("{other:?} is not an artwork task"),
        }
    }

    async fn execute_hashing(&self, task: Task) -> Result<(), HashingError> {
        let Task { kind, priority } = task;
        match kind {
            TaskKind::HashBook { book_id } => {
                self.hashing
                    .hash_and_persist(&book_id, BookHash::File)
                    .await
            }
            TaskKind::HashBookKoreader { book_id } => {
                self.hashing
                    .hash_and_persist(&book_id, BookHash::Koreader)
                    .await
            }
            TaskKind::FindBooksWithMissingPageHash { library_id } => {
                self.hashing
                    .find_books_with_missing_page_hash(&library_id, priority)
                    .await
            }
            TaskKind::HashBookPages { book_id } => {
                self.hashing.hash_pages_and_persist(&book_id).await
            }
            other => unreachable!("{other:?} is not a hashing task"),
        }
    }

    async fn execute_metadata(&self, task: Task) -> Result<(), MetadataError> {
        let Task { kind, priority } = task;
        match kind {
            TaskKind::RefreshBookMetadata {
                book_id,
                capabilities,
                ..
            } => {
                self.metadata
                    .refresh_book_metadata(&book_id, &capabilities, priority)
                    .await
            }
            TaskKind::RefreshSeriesMetadata { series_id } => {
                self.metadata
                    .refresh_series_metadata(&series_id, priority)
                    .await
            }
            TaskKind::AggregateSeriesMetadata { series_id } => {
                self.metadata.aggregate_series_metadata(&series_id).await
            }
            other => unreachable!("{other:?} is not a metadata task"),
        }
    }

    async fn execute_conversion(&self, task: Task) -> Result<(), ConversionError> {
        let Task { kind, priority } = task;
        match kind {
            TaskKind::FindBooksToConvert { library_id } => {
                self.converter
                    .find_books_to_convert(&library_id, priority)
                    .await
            }
            TaskKind::ConvertBook { book_id, .. } => self.converter.convert_to_cbz(&book_id).await,
            TaskKind::RepairExtension { book_id, .. } => {
                self.converter.repair_extension(&book_id).await
            }
            TaskKind::FindDuplicatePagesToDelete { library_id } => {
                self.converter
                    .find_duplicate_pages_to_delete(&library_id, priority)
                    .await
            }
            TaskKind::RemoveHashedPages { book_id, pages } => {
                self.converter
                    .remove_hashed_pages(&book_id, &pages, priority)
                    .await
            }
            other => unreachable!("{other:?} is not a conversion task"),
        }
    }
}

impl TaskExecutor for TaskHandler {
    async fn execute(&self, task: Task, stop: CancellationToken) -> Result<(), TaskFailure> {
        match task.kind {
            TaskKind::EmptyTrash { library_id } => self
                .library_content
                .empty_trash(&library_id)
                .await
                .map_err(failure),
            TaskKind::ScanLibrary {
                library_id,
                scan_deep,
            } => self.scan_library(&library_id, scan_deep, &stop).await,
            TaskKind::AnalyzeBook { book_id, .. } => self
                .book_analysis
                .analyze_and_persist(&book_id, task.priority)
                .await
                .map_err(analysis_failure),
            TaskKind::GenerateBookThumbnail { .. }
            | TaskKind::RefreshBookLocalArtwork { .. }
            | TaskKind::RefreshSeriesLocalArtwork { .. }
            | TaskKind::FindBookThumbnailsToRegenerate { .. } => {
                self.execute_artwork(task).await.map_err(artwork_failure)
            }
            TaskKind::HashBook { .. }
            | TaskKind::HashBookKoreader { .. }
            | TaskKind::FindBooksWithMissingPageHash { .. }
            | TaskKind::HashBookPages { .. } => {
                self.execute_hashing(task).await.map_err(hashing_failure)
            }
            TaskKind::RefreshBookMetadata { .. }
            | TaskKind::RefreshSeriesMetadata { .. }
            | TaskKind::AggregateSeriesMetadata { .. } => {
                self.execute_metadata(task).await.map_err(metadata_failure)
            }
            TaskKind::RebuildIndex => index_result(self.search_index.rebuild(&stop).await),
            TaskKind::UpgradeIndex => index_result(self.search_index.upgrade(&stop).await),
            TaskKind::ImportBook { request } => self
                .importer
                .import(request, task.priority)
                .await
                .map_err(import_failure),
            TaskKind::DeleteBook { book_id } => self
                .deletion
                .delete_book_file(&book_id)
                .await
                .map_err(deletion_failure),
            TaskKind::DeleteSeries { series_id } => self
                .deletion
                .delete_series_files(&series_id)
                .await
                .map_err(deletion_failure),
            TaskKind::FindBooksToConvert { .. }
            | TaskKind::ConvertBook { .. }
            | TaskKind::RepairExtension { .. }
            | TaskKind::FindDuplicatePagesToDelete { .. }
            | TaskKind::RemoveHashedPages { .. } => self
                .execute_conversion(task)
                .await
                .map_err(conversion_failure),
        }
    }
}
