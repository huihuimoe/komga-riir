//! Kotlin `SearchIndexLifecycle`: brings the search index up to the database; `candidates`
//! searches it for the lists.
//!
//! Every transaction that changes an indexed field also marks the entity pending
//! (`SearchIndexPendingDao::mark`); the updater then indexes the entity as the database holds it,
//! so records may be applied late, twice, or out of order without an older state winning. A
//! record is removed only by `SEQ`, which keeps one marked again meanwhile.
//!
//! A rebuild marks every entity and applies the records to a new generation, which replaces the
//! current one only once it holds them all; until then searches use the current one, and a
//! failed or interrupted rebuild leaves it in place.

mod candidates;
mod documents;

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use komga_db::{Database, DbError, PendingUpdate, SearchIndexPendingDao};
use komga_search::{IndexGeneration, RebuildReason, SearchError, SearchIndex};
use komga_types::SearchEntity;
use thiserror::Error;
use tokio::sync::{Mutex, Notify};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub use self::candidates::FullTextSearch;
use self::documents::current_documents;

/// Records applied per index commit.
const BATCH: u64 = 1000;

#[derive(Debug, Error)]
pub enum SearchIndexError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Index(#[from] SearchError),
    #[error("Search index update panicked: {0}")]
    Panicked(#[from] tokio::task::JoinError),
    /// Only list searches have a budget.
    #[error(transparent)]
    OverBudget(#[from] SearchBudgetError),
    #[error("Interrupted by a shutdown")]
    Interrupted,
}

/// A full-text search needs more than it may use; no part of its result is returned.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum SearchBudgetError {
    #[error("Search matches {hits} items, more than the {limit} a search may hold")]
    TooManyHits { hits: usize, limit: usize },
    #[error("Too many searches are running")]
    Busy,
    #[error("Search ran out of time")]
    TimedOut,
}

/// Wakes the updater once a transaction that marked records has committed.
#[derive(Clone, Default)]
pub struct SearchIndexNotifier(Arc<Notify>);

impl SearchIndexNotifier {
    pub(crate) fn notify(&self) {
        self.0.notify_one();
    }
}

/// Applies pending records to the index, and rebuilds it.
#[derive(Clone)]
pub struct SearchIndexUpdater(Arc<Updater>);

struct Updater {
    db: Database,
    index: Arc<SearchIndex>,
    /// Held while records are applied, so that a rebuild removes only those it applied.
    writing: Mutex<()>,
}

impl SearchIndexUpdater {
    #[must_use]
    pub fn new(db: Database, index: Arc<SearchIndex>) -> Self {
        Self(Arc::new(Updater {
            db,
            index,
            writing: Mutex::new(()),
        }))
    }

    /// Why the index must be rebuilt, if it must.
    #[must_use]
    pub fn rebuild_reason(&self) -> Option<RebuildReason> {
        self.0.index.rebuild_reason()
    }

    /// Applies the records earlier runs left, then returns the updater to spawn, which applies
    /// those of each change `commit_delay` after the first change it is woken for
    /// (`LuceneAsyncCommitter`), so that changes in between share one commit, until `shutdown`.
    /// Await it before serving, so that searches see the changes earlier runs left. Records
    /// that fail stay pending for the next change or start; an update under way when `shutdown`
    /// resolves completes first.
    pub async fn start(
        self,
        notifier: SearchIndexNotifier,
        commit_delay: Duration,
        shutdown: impl Future<Output = ()>,
    ) -> impl Future<Output = ()> {
        self.catch_up().await;
        self.run(notifier, commit_delay, shutdown)
    }

    async fn catch_up(&self) {
        if let Err(error) = self.apply_pending().await {
            warn!(%error, "Failed to update the search index; the changes stay pending");
        }
    }

    async fn run(
        self,
        notifier: SearchIndexNotifier,
        commit_delay: Duration,
        shutdown: impl Future<Output = ()>,
    ) {
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                () = notifier.0.notified() => {}
                () = &mut shutdown => return,
            }
            tokio::select! {
                () = tokio::time::sleep(commit_delay) => {}
                () = &mut shutdown => return,
            }
            self.catch_up().await;
        }
    }

    /// Kotlin `SearchIndexLifecycle.rebuildIndex`, of every entity.
    pub(crate) async fn rebuild(&self, stop: &CancellationToken) -> Result<(), SearchIndexError> {
        let _writing = self.0.writing.lock().await;
        self.rebuild_locked(stop).await
    }

    /// Kotlin `SearchIndexLifecycle.upgradeIndex`: rebuilds an index this build cannot use as is.
    pub(crate) async fn upgrade(&self, stop: &CancellationToken) -> Result<(), SearchIndexError> {
        let _writing = self.0.writing.lock().await;
        if self.rebuild_reason().is_none() {
            return Ok(());
        }
        self.rebuild_locked(stop).await
    }

    /// Stops between batches when `stop` is cancelled; the current generation then stays
    /// current, and the abandoned one is removed when the index is next opened.
    async fn rebuild_locked(&self, stop: &CancellationToken) -> Result<(), SearchIndexError> {
        let started = Instant::now();
        info!("Rebuilding the search index");
        let generation = Arc::new(self.on_index(SearchIndex::new_generation).await?);
        let mut tx = self.0.db.begin_immediate().await?;
        SearchIndexPendingDao::mark_all(&mut tx).await?;
        tx.commit().await?;

        let mut applied = Vec::new();
        for entity in SearchEntity::ALL {
            let mut through = 0;
            loop {
                if stop.is_cancelled() {
                    return Err(SearchIndexError::Interrupted);
                }
                let batch = self.index_batch(&generation, entity, through).await?;
                through = batch.last().map_or(through, |update| update.seq);
                if !is_full(&batch) {
                    break;
                }
            }
            applied.push((entity, through));
        }
        let generation =
            Arc::into_inner(generation).expect("Applied batches no longer hold the generation");
        self.on_index(move |index| index.switch(generation)).await?;

        // Removed only once current, so that records applied to a generation that never became
        // current are applied again.
        let mut tx = self.0.db.begin_immediate().await?;
        for (entity, through) in applied {
            SearchIndexPendingDao::remove_through(&mut tx, entity, through).await?;
        }
        tx.commit().await?;
        info!(elapsed = ?started.elapsed(), "Rebuilt the search index");
        Ok(())
    }

    async fn apply_pending(&self) -> Result<(), SearchIndexError> {
        let _writing = self.0.writing.lock().await;
        let generation = match self.0.index.current() {
            Ok(generation) => generation,
            // The records stay pending for the rebuild.
            Err(SearchError::Unavailable(_)) => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entity in SearchEntity::ALL {
            loop {
                let batch = self.index_batch(&generation, entity, 0).await?;
                if batch.is_empty() {
                    break;
                }
                let seqs: Vec<i64> = batch.iter().map(|update| update.seq).collect();
                let mut tx = self.0.db.begin_immediate().await?;
                SearchIndexPendingDao::remove(&mut tx, &seqs).await?;
                tx.commit().await?;
                debug!(
                    entity = entity.type_name(),
                    count = batch.len(),
                    "Updated the search index"
                );
                if !is_full(&batch) {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Applies the oldest records of `entity` with a `SEQ` above `after` to `generation`, and
    /// returns them.
    async fn index_batch(
        &self,
        generation: &Arc<IndexGeneration>,
        entity: SearchEntity,
        after: i64,
    ) -> Result<Vec<PendingUpdate>, SearchIndexError> {
        let mut tx = self.0.db.begin().await?;
        let pending = SearchIndexPendingDao::find(&mut tx, entity, after, BATCH).await?;
        if pending.is_empty() {
            tx.commit().await?;
            return Ok(pending);
        }
        let updates = current_documents(&mut tx, entity, &pending).await?;
        tx.commit().await?;
        let generation = Arc::clone(generation);
        tokio::task::spawn_blocking(move || generation.apply(&updates)).await??;
        Ok(pending)
    }

    async fn on_index<T: Send + 'static>(
        &self,
        work: impl FnOnce(&SearchIndex) -> Result<T, SearchError> + Send + 'static,
    ) -> Result<T, SearchIndexError> {
        let updater = Arc::clone(&self.0);
        Ok(tokio::task::spawn_blocking(move || work(&updater.index)).await??)
    }
}

fn is_full(batch: &[PendingUpdate]) -> bool {
    u64::try_from(batch.len()).is_ok_and(|length| length == BATCH)
}
