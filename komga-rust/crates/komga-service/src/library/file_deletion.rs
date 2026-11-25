//! Kotlin `BookLifecycle.deleteBookFiles` and `SeriesLifecycle.deleteSeriesFiles`: book and
//! series files deleted on user request. A book file is set aside and only removed once the
//! database records the book as deleted, so a crash before that puts it back.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use komga_db::{
    BookDao, Database, DbError, HistoricalEventDao, LibraryDao, SeriesDao, ThumbnailBookDao,
    ThumbnailSeriesDao,
};
use komga_scheduler::{Task, TaskQueue};
use komga_types::{Book, BookId, FileUrl, HistoricalEvent, LibraryId, SeriesId, ThumbnailType};
use thiserror::Error;
use tokio::task::{self, JoinError};
use tracing::info;

use crate::catalog::search_index::SearchIndexNotifier;
use crate::events::EventBus;
use crate::library::file_operation::{
    FileOperationError, FileOperations, PendingFile, SourceAfter,
};
use crate::library::series::{soft_delete_books, soft_delete_series};

#[derive(Debug, Error)]
pub enum FileDeletionError {
    #[error(transparent)]
    FileOperation(#[from] FileOperationError),
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error("Filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("Filesystem worker failed: {0}")]
    Worker(#[from] JoinError),
}

#[derive(Clone)]
pub struct FileDeletion {
    db: Database,
    tasks: TaskQueue,
    files: FileOperations,
    search: SearchIndexNotifier,
    events: EventBus,
}

impl FileDeletion {
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

    /// Kotlin `BookController.deleteBookFile`: queues the deletion first.
    pub async fn submit_book(&self, id: &BookId) -> Result<(), FileDeletionError> {
        Ok(self
            .tasks
            .submit(&Task::delete_book(id.clone()).highest())
            .await?)
    }

    /// Kotlin `SeriesController.deleteSeriesFile`: queues the deletion first.
    pub async fn submit_series(&self, id: &SeriesId) -> Result<(), FileDeletionError> {
        Ok(self
            .tasks
            .submit(&Task::delete_series(id.clone()).highest())
            .await?)
    }

    /// Kotlin `Task.DeleteBook`: a oneshot book is deleted with its series.
    pub(crate) async fn delete_book_file(&self, id: &BookId) -> Result<(), FileDeletionError> {
        let mut tx = self.db.begin().await?;
        let book = BookDao::find_by_id(&mut tx, id).await?;
        tx.rollback().await?;
        match book {
            Some(book) if book.oneshot => self.delete_series_files(&book.series_id).await,
            Some(book) => self.delete_book(&book).await,
            None => Ok(()),
        }
    }

    /// Deletes the files of every book of the series and its sidecar thumbnails, then its folder
    /// when left empty, and marks the series deleted. A missing or unwritable series path is left
    /// as it is.
    pub(crate) async fn delete_series_files(&self, id: &SeriesId) -> Result<(), FileDeletionError> {
        let mut tx = self.db.begin().await?;
        let series = SeriesDao::find_by_id(&mut tx, id).await?;
        let books = BookDao::find_all_by_series(&mut tx, id).await?;
        let sidecars =
            ThumbnailSeriesDao::find_urls_by_series_and_type(&mut tx, id, ThumbnailType::Sidecar)
                .await?;
        tx.rollback().await?;
        let Some(series) = series else {
            return Ok(());
        };
        // Folder URLs end with a slash, which Kotlin's `Path` drops from the recorded name.
        let path: PathBuf = series
            .url
            .to_path()
            .expect("Series have file URLs")
            .components()
            .collect();
        if !is_writable(path.clone()).await? {
            info!(path = %path.display(), "Cannot delete series folder, path does not exist or is not writable");
            return Ok(());
        }
        for book in &books {
            self.delete_book(book).await?;
        }
        remove_sidecars(sidecars).await?;
        self.remove_empty_folder(&series.library_id, id, &path)
            .await?;
        let mut tx = self.db.begin_immediate().await?;
        if let Some(series) = SeriesDao::find_by_id(&mut tx, id).await? {
            soft_delete_series(&mut tx, &[&series]).await?;
        }
        self.events.commit(tx).await?;
        self.search.notify();
        Ok(())
    }

    /// Deletes the book file and its sidecar thumbnails, then its folder when left empty, and
    /// marks the book deleted. A missing or unwritable book file is left as it is.
    async fn delete_book(&self, book: &Book) -> Result<(), FileDeletionError> {
        let path = book.url.to_path().expect("Books have file URLs");
        if !is_writable(path.clone()).await? {
            info!(path = %path.display(), "Cannot delete book file, path does not exist or is not writable");
            return Ok(());
        }
        let mut tx = self.db.begin().await?;
        let sidecars =
            ThumbnailBookDao::find_urls_by_book_and_type(&mut tx, &book.id, ThumbnailType::Sidecar)
                .await?;
        tx.rollback().await?;
        let file = self
            .files
            .begin(&path, &path, Some(&path), SourceAfter::Keep)
            .await?;
        let result = self.remove_and_record(&file, book).await;
        self.files.finish(file).await?;
        if !result? {
            return Ok(());
        }
        info!(path = %path.display(), "Deleted file");
        remove_sidecars(sidecars).await?;
        let folder = path.parent().expect("A book file has a folder");
        self.remove_empty_folder(&book.library_id, &book.series_id, folder)
            .await
    }

    /// Sets the file aside and records the book deleted with its history, unless a scan changed
    /// the book meanwhile. Whether the book was recorded.
    async fn remove_and_record(
        &self,
        file: &PendingFile,
        book: &Book,
    ) -> Result<bool, FileDeletionError> {
        self.files.set_aside(file).await?;
        let mut tx = self.db.begin_immediate().await?;
        if BookDao::find_by_id(&mut tx, &book.id).await?.as_ref() != Some(book) {
            tx.rollback().await?;
            info!(book = %book.id, "Book changed during the file deletion, keeping the file");
            return Ok(false);
        }
        let event = HistoricalEvent::book_file_deleted(book, "File was deleted by user request");
        HistoricalEventDao::insert(&mut tx, &event).await?;
        soft_delete_books(&mut tx, std::slice::from_ref(book)).await?;
        self.files.commit_removal(&mut tx, file).await?;
        self.events.commit(tx).await?;
        self.search.notify();
        Ok(true)
    }

    /// Removes the series folder when nothing is left in it, except the library root.
    async fn remove_empty_folder(
        &self,
        library_id: &LibraryId,
        series_id: &SeriesId,
        folder: &Path,
    ) -> Result<(), FileDeletionError> {
        let mut read_tx = self.db.begin().await?;
        let root = LibraryDao::find_by_id(&mut read_tx, library_id)
            .await?
            .map(|library| library.root);
        read_tx.rollback().await?;
        if root.as_deref() == Some(folder) {
            return Ok(());
        }
        // Perform directory inspection and removal off the async runtime and outside any DB lock.
        let target = folder.to_path_buf();
        let removed = task::spawn_blocking(move || remove_if_empty(&target)).await??;
        if !removed {
            return Ok(());
        }
        // Immediate: the search index updater, woken by the book deletion, writes concurrently.
        let mut tx = self.db.begin_immediate().await?;
        let reason = "Folder was deleted because it was empty";
        let event = HistoricalEvent::series_folder_deleted(series_id, folder, reason);
        HistoricalEventDao::insert(&mut tx, &event).await?;
        tx.commit().await?;
        info!(path = %folder.display(), "Deleted directory");
        Ok(())
    }
}

fn remove_if_empty(folder: &Path) -> io::Result<bool> {
    if !folder.is_dir() || fs::read_dir(folder)?.next().is_some() {
        return Ok(false);
    }
    match fs::remove_dir(folder) {
        Ok(()) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

async fn remove_sidecars(urls: Vec<FileUrl>) -> Result<(), FileDeletionError> {
    task::spawn_blocking(move || {
        for sidecar in urls.iter().filter_map(FileUrl::to_path) {
            if writable(&sidecar)? {
                fs::remove_file(&sidecar)?;
                info!(path = %sidecar.display(), "Deleted file");
            }
        }
        io::Result::Ok(())
    })
    .await??;
    Ok(())
}

async fn is_writable(path: PathBuf) -> Result<bool, FileDeletionError> {
    Ok(task::spawn_blocking(move || writable(&path)).await??)
}

/// Kotlin `exists() && isWritable()`: `access(W_OK)` for the process, not the mode bits, so
/// another owner's `0755` folder is not writable and root may delete a `0444` file.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn writable(path: &Path) -> io::Result<bool> {
    use rustix::fs::{Access, access};
    use rustix::io::Errno;

    match access(path, Access::WRITE_OK) {
        Ok(()) => Ok(true),
        Err(Errno::NOENT | Errno::ACCESS | Errno::PERM | Errno::ROFS) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Kotlin `exists() && isWritable()`.
#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn writable(path: &Path) -> io::Result<bool> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(!metadata.permissions().readonly()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}
