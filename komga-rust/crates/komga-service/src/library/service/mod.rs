//! Library configuration and database-only deletion use cases.
//!
//! Library CRUD without modifying source files.

mod paths;
mod update;

pub use update::LibraryUpdate;

use std::collections::HashMap;
use std::time::Duration;

use komga_db::{Database, DbError, LibraryDao};
use komga_scheduler::{Task, TaskQueue};
use komga_types::{ContentScope, DomainEvent, KomgaUser, Library, LibraryId};
use thiserror::Error;
use tracing::info;

use crate::BlockingError;
use crate::catalog::search_index::SearchIndexNotifier;
use crate::events::EventBus;
use crate::library::series;
use crate::tasks::emitter;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("Library not found")]
    NotFound,
    #[error("Forbidden")]
    Forbidden,
    #[error("{0}")]
    Invalid(String),
    #[error("Filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Blocking(#[from] BlockingError),
    #[error("Database error: {0}")]
    Db(#[from] DbError),
}

/// Path probes run outside SQLite transactions and off the reactor.
#[derive(Clone)]
pub struct LibraryService {
    db: Database,
    tasks: TaskQueue,
    search: SearchIndexNotifier,
    events: EventBus,
}

impl LibraryService {
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

    pub async fn list(&self, user: &KomgaUser) -> Result<Vec<Library>, LibraryError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        let mut libraries = LibraryDao::find_all(&mut tx, Some(&scope)).await?;
        tx.rollback().await?;
        libraries.sort_by_cached_key(|library| library.name.to_lowercase());
        Ok(libraries)
    }

    pub async fn get(&self, user: &KomgaUser, id: &LibraryId) -> Result<Library, LibraryError> {
        let mut tx = self.db.begin().await?;
        let library = LibraryDao::find_by_id(&mut tx, id)
            .await?
            .ok_or(LibraryError::NotFound)?;
        tx.rollback().await?;
        if ContentScope::for_user(user)
            .library_ids()
            .is_some_and(|ids| !ids.contains(id))
        {
            return Err(LibraryError::Forbidden);
        }
        Ok(library)
    }

    pub async fn create(
        &self,
        user: &KomgaUser,
        library: Library,
    ) -> Result<Library, LibraryError> {
        require_admin(user)?;
        let existing = self.all().await?;
        let library = paths::validate(library, existing).await?;
        let mut tx = self.db.begin_immediate().await?;
        LibraryDao::insert(&mut tx, &library).await?;
        tx.record(DomainEvent::LibraryAdded(library.id.clone()));
        let scan = Task::scan_library(library.id.clone(), /* scan_deep= */ false);
        TaskQueue::stage(&mut tx, &[scan]).await?;
        self.events.commit(tx).await?;
        self.tasks.hand_over(&self.db).await;
        Ok(library)
    }

    pub async fn update(
        &self,
        user: &KomgaUser,
        id: &LibraryId,
        update: LibraryUpdate,
    ) -> Result<(), LibraryError> {
        require_admin(user)?;
        let mut existing = self.all().await?;
        let index = existing
            .iter()
            .position(|library| &library.id == id)
            .ok_or(LibraryError::NotFound)?;
        let current = existing.swap_remove(index);
        let changed = update.into_library(current.clone());
        let changed = paths::validate(changed, existing).await?;
        let mut tx = self.db.begin_immediate().await?;
        LibraryDao::update(&mut tx, &changed).await?;
        let mut tasks = Vec::new();
        if should_rescan(&current, &changed) {
            tasks.push(Task::scan_library(id.clone(), /* scan_deep= */ false));
        }
        tasks.extend(emitter::enabled_option_follow_ups(&mut tx, &current, &changed).await?);
        TaskQueue::stage(&mut tx, &tasks).await?;
        tx.record(DomainEvent::LibraryUpdated(id.clone()));
        self.events.commit(tx).await?;
        self.tasks.hand_over(&self.db).await;
        Ok(())
    }

    pub async fn delete(&self, user: &KomgaUser, id: &LibraryId) -> Result<(), LibraryError> {
        require_admin(user)?;
        let mut tx = self.db.begin_immediate().await?;
        if LibraryDao::find_by_id(&mut tx, id).await?.is_none() {
            return Err(LibraryError::NotFound);
        }
        let deleted = LibraryDao::delete(&mut tx, id).await?;
        series::record_deleted(&mut tx, deleted);
        tx.record(DomainEvent::LibraryDeleted(id.clone()));
        self.events.commit(tx).await?;
        self.search.notify();
        Ok(())
    }

    /// Queues `EmptyTrash`; the response does not wait for it to run.
    pub async fn empty_trash(&self, user: &KomgaUser, id: &LibraryId) -> Result<(), LibraryError> {
        require_admin(user)?;
        self.require_exists(id).await?;
        self.tasks
            .submit(&Task::empty_trash(id.clone()).high())
            .await?;
        Ok(())
    }

    /// Queues `ScanLibrary` ahead of other tasks; the response does not wait for it to run.
    pub async fn scan(
        &self,
        user: &KomgaUser,
        id: &LibraryId,
        scan_deep: bool,
    ) -> Result<(), LibraryError> {
        require_admin(user)?;
        self.require_exists(id).await?;
        self.tasks
            .submit(&Task::scan_library(id.clone(), scan_deep).highest())
            .await?;
        Ok(())
    }

    /// Kotlin `PeriodicScannerController.scanOnStartup`.
    pub async fn scan_on_startup(&self) -> Result<(), LibraryError> {
        let mut tasks = Vec::new();
        for library in self.all().await? {
            if library.scan_on_startup {
                info!(library = %library.name, "Scan on startup");
                tasks.push(Task::scan_library(library.id, /* scan_deep= */ false));
            }
        }
        self.tasks.submit_all(&tasks).await?;
        Ok(())
    }

    /// The libraries scanned periodically, with the time between their scans.
    pub async fn scan_periods(&self) -> Result<HashMap<LibraryId, Duration>, LibraryError> {
        let libraries = self.all().await?;
        Ok(libraries
            .into_iter()
            .filter_map(|library| Some((library.id, library.scan_interval.period()?)))
            .collect())
    }

    /// Kotlin `LibraryScanScheduler`'s task. Replaces a scan of the library that is still queued,
    /// so periods that elapse before it runs add no scan.
    pub async fn scan_periodically(&self, id: &LibraryId) -> Result<(), LibraryError> {
        self.tasks
            .submit(&Task::scan_library(id.clone(), /* scan_deep= */ false))
            .await?;
        Ok(())
    }

    async fn require_exists(&self, id: &LibraryId) -> Result<(), LibraryError> {
        let mut tx = self.db.begin().await?;
        let library = LibraryDao::find_by_id(&mut tx, id).await?;
        tx.rollback().await?;
        library.map(|_| ()).ok_or(LibraryError::NotFound)
    }

    async fn all(&self) -> Result<Vec<Library>, LibraryError> {
        let mut tx = self.db.begin().await?;
        let libraries = LibraryDao::find_all(&mut tx, None).await?;
        tx.rollback().await?;
        Ok(libraries)
    }
}

fn require_admin(user: &KomgaUser) -> Result<(), LibraryError> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(LibraryError::Forbidden)
    }
}

/// Kotlin `checkLibraryShouldRescan`: options that change which files the scan finds.
fn should_rescan(current: &Library, changed: &Library) -> bool {
    current.root != changed.root
        || current.oneshots_directory != changed.oneshots_directory
        || current.scan_cbx != changed.scan_cbx
        || current.scan_pdf != changed.scan_pdf
        || current.scan_epub != changed.scan_epub
        || current.scan_force_modified_time != changed.scan_force_modified_time
        || current.scan_directory_exclusions != changed.scan_directory_exclusions
}
