//! Book file and page hashing: Kotlin `BookLifecycle.hashAndPersist`, `hashKoreaderAndPersist`,
//! and `hashPagesAndPersist`, and the known and unknown duplicate pages.

mod page_hash;
mod pages;

use std::io;
use std::path::Path;

use komga_archive::digest::{self, FileDigest};
use komga_archive::divina::EntryError;
use komga_db::{BookDao, BookHash, Database, DbError, LibraryDao};
use komga_image::ImageError;
use komga_scheduler::{Task, TaskQueue};
use komga_types::{Book, BookId, FileUrl};
use thiserror::Error;
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt;
use tokio::task::{self, JoinError};
use tracing::{info, warn};

use crate::support::file_time;

pub use page_hash::{PageHashError, PageHashService};

#[derive(Debug, Error)]
pub enum HashingError {
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error("Hashing worker failed: {0}")]
    Worker(#[from] JoinError),
    #[error("Book URL {0} is not a local file")]
    InvalidUrl(FileUrl),
    #[error("Cannot read book file: {0}")]
    Io(#[from] io::Error),
    #[error("Cannot read page: {0}")]
    Archive(#[from] EntryError),
    #[error("Cannot rewrite JPEG page: {0}")]
    Image(#[from] ImageError),
}

#[derive(Clone)]
pub struct HashingService {
    db: Database,
    tasks: TaskQueue,
    /// `komga.page-hashing`: how many pages are hashed from each end of a book.
    page_hashing: i32,
}

impl HashingService {
    #[must_use]
    pub fn new(db: Database, tasks: TaskQueue, page_hashing: i32) -> Self {
        Self {
            db,
            tasks,
            page_hashing,
        }
    }

    /// Stores the `kind` hash of a book that has none, when its library enables that hash.
    ///
    /// A hash is stored only for the file version the database records: when the file is another
    /// version before or after hashing, a deep scan is queued to record it, as analysis does; when
    /// a scan recorded another version meanwhile, the result is dropped.
    pub(crate) async fn hash_and_persist(
        &self,
        id: &BookId,
        kind: BookHash,
    ) -> Result<(), HashingError> {
        let mut tx = self.db.begin().await?;
        let Some(book) = BookDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            warn!(book = %id, "Cannot hash book: Book does not exist");
            return Ok(());
        };
        let library = LibraryDao::find_by_id(&mut tx, &book.library_id)
            .await?
            .expect("A book's library exists (foreign key)");
        tx.rollback().await?;
        let enabled = match kind {
            BookHash::File => library.hash_files,
            BookHash::Koreader => library.hash_koreader,
        };
        if !enabled {
            info!(book = %id, ?kind, "Hashing is disabled for the library, skipping");
            return Ok(());
        }
        if !stored(&book, kind).is_empty() {
            info!(book = %id, ?kind, "Book already has a hash, skipping");
            return Ok(());
        }
        info!(book = %id, ?kind, "Hash and persist book");

        let path = book
            .url
            .to_path()
            .ok_or_else(|| HashingError::InvalidUrl(book.url.clone()))?;
        let hash = if is_recorded(&path, &book).await? {
            let hash = compute(&path, kind).await?;
            is_recorded(&path, &book).await?.then_some(hash)
        } else {
            None
        };
        let Some(hash) = hash else {
            warn!(book = %id, "Book file is not the recorded version, scanning the library");
            self.tasks
                .submit(&Task::scan_library(library.id, true))
                .await?;
            return Ok(());
        };

        let mut tx = self.db.begin_immediate().await?;
        let Some(mut current) = BookDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            return Ok(());
        };
        let same_version = (current.file_last_modified, current.file_size)
            == (book.file_last_modified, book.file_size);
        if !same_version || !stored(&current, kind).is_empty() {
            tx.rollback().await?;
            warn!(book = %id, "Book changed during hashing, dropping the result");
            return Ok(());
        }
        match kind {
            BookHash::File => current.file_hash = hash,
            BookHash::Koreader => current.file_hash_koreader = hash,
        }
        BookDao::update(&mut tx, &current).await?;
        tx.commit().await?;
        Ok(())
    }
}

fn stored(book: &Book, kind: BookHash) -> &str {
    match kind {
        BookHash::File => &book.file_hash,
        BookHash::Koreader => &book.file_hash_koreader,
    }
}

async fn is_recorded(path: &Path, book: &Book) -> io::Result<bool> {
    Ok(file_time::is_recorded(&fs::metadata(path).await?, book))
}

async fn compute(path: &Path, kind: BookHash) -> Result<String, HashingError> {
    match kind {
        BookHash::File => Ok(file_hash(path).await?),
        // A dozen seeks for 1 KiB each gain nothing from io_uring.
        BookHash::Koreader => {
            let path = path.to_owned();
            Ok(task::spawn_blocking(move || digest::koreader_hash(&path)).await??)
        }
    }
}

/// Kotlin `Hasher.computeHash(Path)`, read sequentially.
pub(crate) async fn file_hash(path: &Path) -> io::Result<String> {
    let mut file = File::open(path).await?;
    let mut digest = FileDigest::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            return Ok(digest.finish());
        }
        digest.update(&buffer[..read]);
    }
}
