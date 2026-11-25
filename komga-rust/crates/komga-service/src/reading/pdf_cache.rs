//! The bytes of recently read PDF books. hayro parses only from a whole file in memory, so
//! without them every page read would read the whole file again.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use komga_archive::pdf::{self, PdfDocument, PdfError};
use komga_types::BookId;

use super::file_cache::FileCache;

/// Files of more than about 97% of this are read for each page, as quick_cache does not keep
/// them.
const CAPACITY_BYTES: u64 = 512 * 1024 * 1024;

/// How long cached files stay in memory once unread: page after page is seconds apart, and an
/// idle server should not keep them.
pub(super) const IDLE_EXPIRY: Duration = Duration::from_secs(10 * 60);

#[derive(Clone)]
pub(super) struct PdfCache {
    files: FileCache<Arc<Vec<u8>>>,
}

impl PdfCache {
    pub(super) fn new() -> Self {
        Self {
            files: FileCache::new(64, CAPACITY_BYTES),
        }
    }

    /// Opens the file of `book` at `path`, from the cache when its modification time and size
    /// are unchanged. Concurrent opens of an uncached version wait for a single read; only
    /// files that parse are kept. Blocking.
    ///
    /// # Errors
    /// See [`pdf::read_file`] and [`PdfDocument::from_bytes`].
    pub(super) fn open(&self, book: &BookId, path: &Path) -> Result<PdfDocument, PdfError> {
        self.files.get_or_load(
            book,
            path,
            |bytes| PdfDocument::from_bytes(Arc::clone(bytes)),
            |file| {
                let bytes = Arc::new(pdf::read_file(file)?);
                let document = PdfDocument::from_bytes(Arc::clone(&bytes))?;
                let weight = bytes.len() as u64;
                Ok((bytes, weight, document))
            },
        )
    }

    /// Drops the files no page of which was read for `idle`.
    pub(super) fn evict_idle(&self, idle: Duration) {
        self.files.evict_idle(idle);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::thread;
    use std::time::Duration;

    use komga_types::BookId;

    use super::PdfCache;

    fn fixture(name: &str) -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../komga-archive/tests/fixtures/pdf/"
        );
        fs::read(format!("{path}{name}")).expect("Fixture")
    }

    /// Overwrites the book with as many bytes that are not a PDF, keeping its modification time,
    /// so that only the cache can still answer.
    fn corrupt_keeping_version(path: &Path) {
        let file = fs::File::options().write(true).open(path).expect("Book");
        let metadata = file.metadata().expect("Book metadata");
        let modified = metadata.modified().expect("Modification time");
        let len = usize::try_from(metadata.len()).expect("Small book");
        fs::write(path, vec![b'x'; len]).expect("Overwritten book");
        file.set_modified(modified)
            .expect("Restored modification time");
    }

    /// Cached bytes answer while the file keeps its time and size; any other version is read again.
    #[test]
    fn unchanged_files_are_not_read_again() {
        let dir = komga_test_support::tempdir().expect("Temporary directory");
        let path = dir.path().join("book.pdf");
        fs::write(&path, fixture("pages.pdf")).expect("Written book");
        let (cache, book) = (PdfCache::new(), BookId::from("book".to_owned()));
        let pages = |cache: &PdfCache| cache.open(&book, &path).map(|pdf| pdf.page_sizes().len());

        assert_eq!(pages(&cache).ok(), Some(4));
        corrupt_keeping_version(&path);
        assert_eq!(pages(&cache).ok(), Some(4));
        // Another book at the same path is not served this book's bytes.
        let other = cache.open(&BookId::from("other".to_owned()), &path).err();
        assert!(other.is_some(), "Other book served from the cache");

        // A new modification time alone is a new version.
        let file = fs::File::options().write(true).open(&path).expect("Book");
        let modified = file.metadata().and_then(|metadata| metadata.modified());
        file.set_modified(modified.expect("Modification time") + Duration::from_secs(1))
            .expect("Modification time");
        assert!(pages(&cache).is_err(), "Stale bytes served");
        fs::write(&path, fixture("komga.pdf")).expect("Replaced book");
        assert_eq!(pages(&cache).ok(), Some(1));
    }

    /// A read keeps the file cached for the idle time from then; past it, the file is dropped.
    #[test]
    fn files_left_unread_are_dropped() {
        let dir = komga_test_support::tempdir().expect("Temporary directory");
        let path = dir.path().join("book.pdf");
        fs::write(&path, fixture("pages.pdf")).expect("Written book");
        let (cache, book) = (PdfCache::new(), BookId::from("book".to_owned()));
        let pages = |cache: &PdfCache| cache.open(&book, &path).map(|pdf| pdf.page_sizes().len());

        assert_eq!(pages(&cache).ok(), Some(4));
        corrupt_keeping_version(&path);
        thread::sleep(Duration::from_millis(300));
        assert_eq!(pages(&cache).ok(), Some(4));
        // Idle for less than 200 ms since the last read, though cached 300 ms ago.
        cache.evict_idle(Duration::from_millis(200));
        assert_eq!(pages(&cache).ok(), Some(4));
        cache.evict_idle(Duration::ZERO);
        assert!(pages(&cache).is_err(), "Dropped bytes served");
    }
}
