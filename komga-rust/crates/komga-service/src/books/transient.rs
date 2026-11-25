//! Kotlin `TransientBookLifecycle`: books in a folder outside every library, scanned, analyzed
//! and read in memory so that an admin can review them before importing. Nothing here writes a
//! file or a library entity.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use jiff::Timestamp;
use komga_db::{Database, DbError, LibraryDao};
use komga_types::{Book, BookId, BookPage, KomgaUser, Library, Media, MediaStatus, SeriesId};
use thiserror::Error;

use crate::books::analysis::{self, BookAnalysisError};
use crate::books::importing::is_in_library;
use crate::library::scanning::{ScanError, scan_root_folder};
use crate::metadata::transient;
use crate::reading::{self, PageContent, ReadingError, ReadingService};
use crate::support::blocking::{self, BlockingError};

/// Kotlin's cache drops a book an hour after it was last used.
const EXPIRY: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Error)]
pub enum TransientBookError {
    #[error("Forbidden")]
    Forbidden,
    /// Kotlin `ERR_1016`, also for a relative path, which Kotlin resolves against its working
    /// directory.
    #[error(transparent)]
    Scan(#[from] ScanError),
    /// Kotlin `PathContainedInPath` (`ERR_1017`).
    #[error("Cannot scan a folder of an existing library")]
    InLibrary,
    #[error("Transient book not found")]
    NotFound,
    #[error(transparent)]
    Analysis(#[from] BookAnalysisError),
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Blocking(#[from] BlockingError),
}

#[derive(Debug, Clone)]
pub struct TransientBook {
    pub book: Book,
    /// Unknown until analyzed.
    pub media: Media,
    /// When `media` was recorded, in epoch milliseconds (Kotlin `getBookLastModified`).
    pub last_modified: i64,
    pub number: Option<f32>,
    pub series_id: Option<SeriesId>,
}

impl TransientBook {
    fn new(book: Book) -> Self {
        Self {
            book,
            media: Media::failed(MediaStatus::Unknown, None),
            last_modified: Timestamp::now().as_millisecond(),
            number: None,
            series_id: None,
        }
    }

    /// The pages, PDF pages as rendered.
    #[must_use]
    pub fn pages(&self) -> Vec<BookPage> {
        reading::pages_as_read(self.media.clone())
    }
}

#[derive(Clone)]
pub struct TransientBookService {
    db: Database,
    reading: ReadingService,
    /// `komga.epub-divina-letter-count-threshold`.
    epub_letter_count_threshold: i32,
    books: Arc<Mutex<HashMap<BookId, (TransientBook, Instant)>>>,
}

impl TransientBookService {
    #[must_use]
    pub fn new(db: Database, reading: ReadingService, epub_letter_count_threshold: i32) -> Self {
        Self {
            db,
            reading,
            epub_letter_count_threshold,
            books: Arc::default(),
        }
    }

    /// The books under `path`, which must not be in a library, ordered by path.
    pub async fn scan(
        &self,
        user: &KomgaUser,
        path: String,
    ) -> Result<Vec<TransientBook>, TransientBookError> {
        check_admin(user)?;
        let mut tx = self.db.begin().await?;
        let roots = LibraryDao::find_all(&mut tx, None)
            .await?
            .into_iter()
            .map(|library| library.root)
            .collect();
        tx.rollback().await?;
        let mut books = blocking::run(move || scan_outside(PathBuf::from(path), roots)).await??;
        books.sort_by_cached_key(|book| book.url.to_path());
        let books: Vec<_> = books.into_iter().map(TransientBook::new).collect();
        let mut stored = self.stored();
        for book in &books {
            stored.insert(book.book.id.clone(), (book.clone(), Instant::now()));
        }
        Ok(books)
    }

    /// Analyzes the book again and looks up its number and series.
    pub async fn analyze(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<TransientBook, TransientBookError> {
        let mut book = self.find(user, id)?;
        let path = book
            .book
            .url
            .to_path()
            .expect("Scanned books have file URLs");
        let threshold = self.epub_letter_count_threshold;
        book.media = blocking::run(move || analysis::analyze(&path, true, threshold)).await??;
        book.last_modified = Timestamp::now().as_millisecond();
        (book.number, book.series_id) =
            transient::number_and_series(&self.db, &self.reading, &book.book, &book.media).await?;
        self.stored()
            .insert(id.clone(), (book.clone(), Instant::now()));
        Ok(book)
    }

    pub fn find(&self, user: &KomgaUser, id: &BookId) -> Result<TransientBook, TransientBookError> {
        check_admin(user)?;
        let mut stored = self.stored();
        let (book, accessed) = stored.get_mut(id).ok_or(TransientBookError::NotFound)?;
        *accessed = Instant::now();
        Ok(book.clone())
    }

    /// Page `number` (from 1), as stored.
    pub async fn read_page(
        &self,
        book: TransientBook,
        number: i32,
    ) -> Result<PageContent, ReadingError> {
        self.reading
            .read_transient_page(book.book, book.media, number)
            .await
    }

    /// The stored books, without those that expired.
    fn stored(&self) -> MutexGuard<'_, HashMap<BookId, (TransientBook, Instant)>> {
        let mut stored = self.books.lock().expect("No panic while holding the lock");
        stored.retain(|_, (_, accessed)| accessed.elapsed() < EXPIRY);
        stored
    }
}

fn check_admin(user: &KomgaUser) -> Result<(), TransientBookError> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(TransientBookError::Forbidden)
    }
}

/// The books under `path`, refused when it is in one of the library `roots`.
fn scan_outside(path: PathBuf, roots: Vec<PathBuf>) -> Result<Vec<Book>, TransientBookError> {
    if !path.is_absolute() || !path.exists() {
        return Err(ScanError::DirectoryNotFound(path).into());
    }
    if is_in_library(&path, &roots) {
        return Err(TransientBookError::InLibrary);
    }
    let scan = scan_root_folder(&Library::new(String::new(), path))?;
    Ok(scan
        .series
        .into_iter()
        .flat_map(|(_, books)| books)
        .collect())
}
