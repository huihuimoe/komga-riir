//! Kotlin `LibraryContentLifecycle`; the scan itself lives in `library_scan`.

use std::io;

use komga_db::{Database, DbError, LibraryDao};
use komga_scheduler::TaskQueue;
use komga_types::{Library, LibraryId};
use thiserror::Error;
use tokio::task::JoinError;
use tracing::{info, warn};

use super::file_system_scanner::ScanError;
use crate::catalog::search_index::SearchIndexNotifier;
use crate::catalog::{collections, read_lists};
use crate::events::EventBus;
use crate::library::series::{self, SeriesError};
use crate::settings::{ServerSettings, SettingsError};
use crate::tasks::emitter;

#[derive(Debug, Error)]
pub enum LibraryContentError {
    #[error(transparent)]
    Scan(#[from] ScanError),
    #[error("Filesystem worker failed: {0}")]
    Worker(#[from] JoinError),
    #[error(transparent)]
    Series(#[from] SeriesError),
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("Cannot hash book file: {0}")]
    Hash(io::Error),
    #[error("Interrupted by a shutdown")]
    Interrupted,
}

#[derive(Clone)]
pub struct LibraryContentService {
    pub(super) db: Database,
    pub(super) tasks: TaskQueue,
    pub(super) search: SearchIndexNotifier,
    pub(super) events: EventBus,
}

impl LibraryContentService {
    #[must_use]
    pub fn new(
        db: Database,
        tasks: TaskQueue,
        search: SearchIndexNotifier,
        events: EventBus,
    ) -> Self {
        Self {
            db,
            tasks,
            search,
            events,
        }
    }

    /// Permanently deletes the library's soft-deleted series and books from the database, then
    /// renumbers the books of series that lost some; a library deleted since the task was queued
    /// is skipped.
    pub async fn empty_trash(&self, id: &LibraryId) -> Result<(), LibraryContentError> {
        let mut tx = self.db.begin_immediate().await?;
        if LibraryDao::find_by_id(&mut tx, id).await?.is_none() {
            tx.rollback().await?;
            warn!(library = %id, "Cannot empty trash: Library does not exist");
            return Ok(());
        }
        info!(library = %id, "Empty trash for library");
        let mut refreshes = Vec::new();
        let (deleted, renumbered) = LibraryDao::delete_trash(&mut tx, id).await?;
        series::record_deleted(&mut tx, deleted);
        for series_id in renumbered {
            refreshes.extend(series::sort_books(&mut tx, &series_id).await?);
        }
        TaskQueue::stage(&mut tx, &refreshes).await?;
        self.events.commit(tx).await?;
        self.search.notify();
        self.tasks.hand_over(&self.db).await;
        self.cleanup_empty_sets().await
    }

    /// Queues the follow-ups of a successful scan of `library`.
    pub(crate) async fn submit_scan_follow_ups(
        &self,
        library: &Library,
    ) -> Result<(), LibraryContentError> {
        let mut tx = self.db.begin().await?;
        let tasks = emitter::scan_follow_ups(&mut tx, library).await?;
        tx.rollback().await?;
        self.tasks.submit_all(&tasks).await?;
        Ok(())
    }

    pub(super) async fn cleanup_empty_sets(&self) -> Result<(), LibraryContentError> {
        let settings = ServerSettings::load(&self.db).await?;
        if settings.delete_empty_collections {
            collections::delete_empty(&self.db, &self.search, &self.events).await?;
        }
        if settings.delete_empty_read_lists {
            read_lists::delete_empty(&self.db, &self.search, &self.events).await?;
        }
        Ok(())
    }
}
