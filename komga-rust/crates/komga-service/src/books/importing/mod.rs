//! Kotlin `BookImporter`: a file from outside every library becomes a book of a series, copied,
//! moved or linked next to it with its sidecars, optionally upgrading a book of the series. The
//! book file goes through a recorded file operation that also sets the upgraded file aside, so a
//! failure or a crash leaves the source and the upgraded book in place and no half-imported book.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use komga_db::{
    BookDao, Database, DbError, DbTransaction, LibraryDao, RestoreDao, SeriesDao, SidecarDao,
};
use komga_scheduler::{Task, TaskQueue};
use komga_types::{
    Book, BookId, BookImport, BookRef, CopyMode, DomainEvent, FileUrl, ImportRequest, Series,
    Sidecar,
};
use thiserror::Error;
use tokio::task::{self, JoinError};
use tracing::warn;

use crate::catalog::search_index::SearchIndexNotifier;
use crate::events::EventBus;
use crate::library::file_operation::{
    FileOperationError, FileOperations, PendingFile, SourceAfter,
};
use crate::library::scanning::scan_file;
use crate::library::series::{self, SeriesError};

use sidecars::{book_sidecars, sidecar_files, write_sidecar};

mod sidecars;

/// The `Display` of the coded errors is their Kotlin code, which the failure event carries.
#[derive(Debug, Error)]
pub enum ImportError {
    /// Also for a relative path, which Kotlin resolves against its working directory.
    #[error("ERR_1018")]
    SourceNotFound,
    #[error("A book can only be imported into a oneshot series to upgrade its book")]
    Oneshot,
    #[error("ERR_1019")]
    InLibrary,
    /// The book to upgrade belongs to another series.
    #[error("ERR_1020")]
    NotInSeries,
    #[error("ERR_1021")]
    TargetExists,
    #[error("ERR_1022")]
    NotScanned,
    #[error(transparent)]
    FileOperation(FileOperationError),
    #[error(transparent)]
    Series(#[from] SeriesError),
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error("Filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("Filesystem worker failed: {0}")]
    Worker(#[from] JoinError),
}

impl From<FileOperationError> for ImportError {
    fn from(error: FileOperationError) -> Self {
        match error {
            FileOperationError::TargetExists(_) => Self::TargetExists,
            error => Self::FileOperation(error),
        }
    }
}

#[derive(Clone)]
pub struct BookImporter {
    db: Database,
    tasks: TaskQueue,
    files: FileOperations,
    search: SearchIndexNotifier,
    events: EventBus,
}

impl BookImporter {
    #[must_use]
    pub fn new(
        db: Database,
        tasks: TaskQueue,
        search: SearchIndexNotifier,
        events: EventBus,
    ) -> Self {
        Self {
            files: FileOperations::new(db.clone()),
            db,
            tasks,
            search,
            events,
        }
    }

    /// Queues the imports first in line; each is checked when it runs.
    pub async fn submit(&self, requests: Vec<ImportRequest>) -> Result<(), DbError> {
        let tasks: Vec<Task> = requests
            .into_iter()
            .map(|request| Task::import_book(request).highest())
            .collect();
        self.tasks.submit_all(&tasks).await
    }

    /// Imports the file and queues the analysis of the book with the import. Admins are told the outcome, except
    /// when the series no longer exists, which Kotlin only logs.
    pub async fn import(&self, request: ImportRequest, priority: i32) -> Result<(), ImportError> {
        let mut tx = self.db.begin().await?;
        let series = SeriesDao::find_by_id(&mut tx, &request.series_id).await?;
        tx.rollback().await?;
        let Some(series) = series else {
            warn!(series = %request.series_id, "Cannot import a book: the series does not exist");
            return Ok(());
        };
        let result = self.import_into(&request, &series, priority).await;
        self.events.publish(DomainEvent::BookImported(BookImport {
            book_id: result.as_ref().ok().cloned(),
            source_file: request.source_file.clone(),
            message: result.as_ref().err().map(ToString::to_string),
        }));
        result?;
        Ok(())
    }

    async fn import_into(
        &self,
        request: &ImportRequest,
        series: &Series,
        priority: i32,
    ) -> Result<BookId, ImportError> {
        let source = PathBuf::from(&request.source_file);
        if !source.is_absolute() || !tokio::fs::try_exists(&source).await.unwrap_or(false) {
            return Err(ImportError::SourceNotFound);
        }
        let upgraded = self.upgraded_book(request, series).await?;
        if series.oneshot && upgraded.is_none() {
            return Err(ImportError::Oneshot);
        }
        let mut tx = self.db.begin().await?;
        let roots: Vec<PathBuf> = LibraryDao::find_all(&mut tx, None)
            .await?
            .into_iter()
            .map(|library| library.root)
            .collect();
        tx.rollback().await?;
        let path = source.clone();
        if task::spawn_blocking(move || is_in_library(&path, &roots)).await? {
            return Err(ImportError::InLibrary);
        }
        // A oneshot series is its book file.
        let series_path = series.url.to_path().expect("Series have file URLs");
        let directory = if series.oneshot {
            series_path.parent().expect("Absolute files have parents")
        } else {
            &series_path
        };
        let target = directory.join(target_name(&source, request.destination_name.as_deref()));
        let source_after = match request.copy_mode {
            CopyMode::Move => SourceAfter::Remove,
            CopyMode::Copy | CopyMode::Hardlink => SourceAfter::Keep,
        };
        let replaced = upgraded.as_ref().map(book_path);
        let mut file = self
            .files
            .begin(&source, &target, replaced.as_deref(), source_after)
            .await?;
        let result = self
            .publish_and_record(&mut file, request, series, upgraded.as_ref(), priority)
            .await;
        self.files.finish(file).await?;
        let (book_id, leftovers) = result?;
        remove_leftovers(leftovers).await?;
        Ok(book_id)
    }

    /// The book of the series to upgrade; Kotlin imports a new book when it no longer exists.
    async fn upgraded_book(
        &self,
        request: &ImportRequest,
        series: &Series,
    ) -> Result<Option<Book>, ImportError> {
        let Some(book_id) = &request.upgrade_book_id else {
            return Ok(None);
        };
        let mut tx = self.db.begin().await?;
        let book = BookDao::find_by_id(&mut tx, book_id).await?;
        tx.rollback().await?;
        match book {
            Some(book) if book.series_id != series.id => Err(ImportError::NotInSeries),
            book => Ok(book),
        }
    }

    /// Writes the book and its sidecars next to the series and records the book; on failure the
    /// caller's `finish` undoes the book file. Returns the sidecars to remove once finished: the
    /// moved ones and those of the upgraded book that were not replaced.
    async fn publish_and_record(
        &self,
        file: &mut PendingFile,
        request: &ImportRequest,
        series: &Series,
        upgraded: Option<&Book>,
        priority: i32,
    ) -> Result<(BookId, Vec<PathBuf>), ImportError> {
        let link = request.copy_mode != CopyMode::Copy;
        let (source, temporary) = (file.source().to_owned(), file.temporary().to_owned());
        if !(link
            && task::spawn_blocking(move || fs::hard_link(source, temporary))
                .await?
                .is_ok())
        {
            file.copy_source().await?;
        }
        self.files.publish(file).await?;
        let (source, target) = (file.source().to_owned(), file.target().to_owned());
        let destination_name = request.destination_name.clone();
        let upgraded_path = upgraded.map(book_path);
        let moved = request.copy_mode == CopyMode::Move;
        let (sidecars, leftovers) = task::spawn_blocking(move || {
            // Listed before the new sidecars may replace them.
            let mut old = match &upgraded_path {
                Some(path) => sidecar_files(path)?,
                None => Vec::new(),
            };
            let mut sidecars = Vec::new();
            for sidecar in book_sidecars(&source, &target, destination_name.as_deref())? {
                sidecars.push(write_sidecar(&sidecar, link)?);
                old.retain(|path| *path != sidecar.target);
                if moved {
                    old.push(sidecar.source);
                }
            }
            io::Result::Ok((sidecars, old))
        })
        .await??;
        let target = file.target().to_owned();
        let library_id = series.library_id.clone();
        let book = task::spawn_blocking(move || scan_file(&target, &library_id))
            .await?
            .ok_or(ImportError::NotScanned)?;
        let book = Book {
            oneshot: series.oneshot,
            ..book
        };
        let (mut tx, mut tasks) = self
            .record(file, series, &book, &sidecars, upgraded, &leftovers)
            .await?;
        tasks.push(
            Task::analyze_book(book.id.clone(), series.id.clone()).with_priority(priority + 1),
        );
        TaskQueue::stage(&mut tx, &tasks).await?;
        self.events.commit(tx).await?;
        self.search.notify();
        self.tasks.hand_over(&self.db).await;
        Ok((book.id, leftovers))
    }

    /// Adds the book to its series with the sidecars and hands the upgraded book over to it,
    /// marking the file operation committed, in a transaction left to commit with the returned
    /// tasks.
    async fn record(
        &self,
        file: &PendingFile,
        series: &Series,
        book: &Book,
        sidecars: &[Sidecar],
        upgraded: Option<&Book>,
        leftovers: &[PathBuf],
    ) -> Result<(DbTransaction, Vec<Task>), ImportError> {
        let mut tx = self.db.begin_immediate().await?;
        series::add_books(&mut tx, series, std::slice::from_ref(book)).await?;
        for sidecar in sidecars {
            SidecarDao::save(&mut tx, &series.library_id, sidecar).await?;
        }
        if let Some(upgraded) = upgraded {
            let urls: Vec<FileUrl> = leftovers.iter().map(|path| FileUrl::file(path)).collect();
            let urls: Vec<&FileUrl> = urls.iter().collect();
            SidecarDao::delete_by_library_and_urls(&mut tx, &series.library_id, &urls).await?;
            RestoreDao::upgrade_book(&mut tx, &upgraded.id, &book.id).await?;
            tx.record(DomainEvent::BookDeleted(BookRef::from(upgraded)));
            if series.oneshot {
                SeriesDao::update_file(&mut tx, &series.id, &book.url, book.file_last_modified)
                    .await?;
            }
        }
        self.files.commit(&mut tx, file).await?;
        let mut tasks = series::sort_books(&mut tx, &series.id).await?;
        if !sidecars.is_empty() {
            tasks.push(Task::refresh_book_local_artwork(book.id.clone()));
        }
        Ok((tx, tasks))
    }
}

async fn remove_leftovers(leftovers: Vec<PathBuf>) -> Result<(), JoinError> {
    task::spawn_blocking(move || {
        for path in leftovers {
            if let Err(error) = fs::remove_file(&path) {
                warn!(path = %path.display(), %error, "Could not remove a sidecar");
            }
        }
    })
    .await
}

fn book_path(book: &Book) -> PathBuf {
    book.url.to_path().expect("Books have file URLs")
}

/// Whether `path` is in one of the library `roots`. Paths are compared as resolved, so a symbolic
/// link does not hide a library folder.
pub(crate) fn is_in_library(path: &Path, roots: &[PathBuf]) -> bool {
    let resolve = |path: &Path| path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path = resolve(path);
    roots.iter().any(|root| path.starts_with(resolve(root)))
}

/// The source file name, or `destination_name` with the source extension.
fn target_name(source: &Path, destination_name: Option<&str>) -> PathBuf {
    let name = source.file_name().expect("Existing files have names");
    let Some(destination_name) = destination_name else {
        return PathBuf::from(name);
    };
    let extension = source.extension().unwrap_or_default().to_string_lossy();
    let named = PathBuf::from(format!("{destination_name}.{extension}"));
    // Kotlin keeps the last component of the name.
    PathBuf::from(named.file_name().unwrap_or(name))
}
