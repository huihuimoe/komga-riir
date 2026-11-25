//! Bounded, version-keyed, idle-expiring cache shared by whole-file book readers (PDF and MOBI).

use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use komga_types::BookId;
use quick_cache::sync::{Cache, DefaultLifecycle, GuardResult};
use quick_cache::{DefaultHashBuilder, OptionsBuilder, Weighter};

/// A book at one file version: a changed file misses, and its old entry ages out unused.
#[derive(Clone, PartialEq, Eq, Hash)]
struct FileVersion {
    book: BookId,
    modified: SystemTime,
    len: u64,
}

struct CachedEntry<T> {
    value: T,
    weight: u64,
    /// Milliseconds from [`FileCache::epoch`].
    last_used: AtomicU64,
}

#[derive(Clone)]
struct EntryWeighter;

impl<T> Weighter<FileVersion, Arc<CachedEntry<T>>> for EntryWeighter {
    fn weight(&self, _version: &FileVersion, entry: &Arc<CachedEntry<T>>) -> u64 {
        entry.weight.max(1)
    }
}

pub(super) struct FileCache<T> {
    entries: Arc<Cache<FileVersion, Arc<CachedEntry<T>>, EntryWeighter>>,
    epoch: Instant,
}

impl<T> Clone for FileCache<T> {
    fn clone(&self) -> Self {
        Self {
            entries: Arc::clone(&self.entries),
            epoch: self.epoch,
        }
    }
}

impl<T> FileCache<T> {
    pub(super) fn new(estimated_items: usize, capacity_bytes: u64) -> Self {
        let options = OptionsBuilder::new()
            // One shard, so the whole capacity is available to a single file.
            .shards(1)
            .estimated_items_capacity(estimated_items)
            .weight_capacity(capacity_bytes)
            .build()
            .expect("Valid cache options");
        Self {
            entries: Arc::new(Cache::with_options(
                options,
                EntryWeighter,
                DefaultHashBuilder::default(),
                DefaultLifecycle::default(),
            )),
            epoch: Instant::now(),
        }
    }

    fn now(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Returns the cached entry for `(book, mtime, len)` via `on_hit`, or loads and caches it via
    /// `on_miss`. Concurrent opens of an uncached version wait for a single load; failed loads are
    /// not cached. Blocking.
    pub(super) fn get_or_load<R, E>(
        &self,
        book: &BookId,
        path: &Path,
        on_hit: impl FnOnce(&T) -> Result<R, E>,
        on_miss: impl FnOnce(File) -> Result<(T, u64, R), E>,
    ) -> Result<R, E>
    where
        E: From<io::Error>,
    {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        let version = FileVersion {
            book: book.clone(),
            modified: metadata.modified()?,
            len: metadata.len(),
        };
        match self.entries.get_value_or_guard(&version, None) {
            GuardResult::Value(cached) => {
                cached.last_used.store(self.now(), Ordering::Relaxed);
                on_hit(&cached.value)
            }
            GuardResult::Guard(guard) => {
                let (value, weight, result) = on_miss(file)?;
                let cached = CachedEntry {
                    value,
                    weight,
                    last_used: AtomicU64::new(self.now()),
                };
                // Rejected only when the entry is too large to keep.
                let _ = guard.insert(Arc::new(cached));
                Ok(result)
            }
            GuardResult::Timeout => unreachable!("No timeout is set"),
        }
    }

    /// Drops entries unread for at least `idle`.
    pub(super) fn evict_idle(&self, idle: Duration) {
        let idle = u64::try_from(idle.as_millis()).unwrap_or(u64::MAX);
        let now = self.now();
        self.entries.retain(|_, cached| {
            now.saturating_sub(cached.last_used.load(Ordering::Relaxed)) < idle
        });
    }
}
