//! Bounded cache of recently parsed MOBI publications (spec §16.4), avoiding repeated whole-file
//! decompression across chapter and image `/resource/{name}` requests.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use komga_archive::mobi::{self, MobiError, MobiPublication};
use komga_types::BookId;

use super::file_cache::FileCache;

/// Total weight capacity for cached virtual MOBI publications (256 MiB).
const CAPACITY_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct MobiCache {
    entries: FileCache<Arc<MobiPublication>>,
}

impl MobiCache {
    pub(super) fn new() -> Self {
        Self {
            entries: FileCache::new(32, CAPACITY_BYTES),
        }
    }

    /// Opens and normalizes the MOBI `book` at `path`, returning a cached publication when its
    /// modification time and size match. Blocking.
    pub(super) fn open(
        &self,
        book: &BookId,
        path: &Path,
    ) -> Result<Arc<MobiPublication>, MobiError> {
        self.entries.get_or_load(
            book,
            path,
            |publication| Ok(Arc::clone(publication)),
            |file| {
                let bytes = mobi::read_file(file)?;
                let publication = Arc::new(MobiPublication::from_bytes(&bytes)?);
                let weight = publication.memory_bytes();
                Ok((Arc::clone(&publication), weight, publication))
            },
        )
    }

    /// Drops publications unread for at least `idle`.
    pub(super) fn evict_idle(&self, idle: Duration) {
        self.entries.evict_idle(idle);
    }
}
