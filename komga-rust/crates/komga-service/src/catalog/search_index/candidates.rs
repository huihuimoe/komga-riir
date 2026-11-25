//! Kotlin `LuceneHelper.searchEntitiesIds`: the hits of a full-text query, written to the
//! candidate table of the transaction that filters and pages them.

use std::sync::Arc;
use std::time::Duration;

use komga_db::{Database, DbTransaction, SearchCandidates};
use komga_search::SearchIndex;
use komga_types::SearchEntity;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task;
use tokio::time::{self, Instant};

use super::{SearchBudgetError, SearchIndexError};

/// Hit IDs read from the index and written to the table at a time.
const BATCH: usize = 500;

/// What full-text searches may use; one that needs more fails instead of answering in part.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SearchBudget {
    /// Bounds the temporary disk space of one search: a candidate row and its index entry take
    /// about 50 bytes.
    pub(crate) max_hits: usize,
    /// For waiting for a slot, searching the index and writing the candidates.
    pub(crate) time_limit: Duration,
    /// Searches holding candidates at once. Each holds a database connection, so this stays
    /// below the pool size to leave connections for other requests.
    pub(crate) concurrency: usize,
}

impl Default for SearchBudget {
    fn default() -> Self {
        Self {
            max_hits: 1_000_000,
            time_limit: Duration::from_secs(30),
            concurrency: 2,
        }
    }
}

/// Searches the index for list queries.
#[derive(Clone)]
pub struct FullTextSearch {
    index: Arc<SearchIndex>,
    permits: Arc<Semaphore>,
    budget: SearchBudget,
}

/// A read transaction and, for a full-text query, its candidates. Dropping it, as a cancelled
/// request does, rolls the transaction back, which removes the candidates.
pub(crate) struct Search {
    pub(crate) transaction: DbTransaction,
    pub(crate) candidates: Option<SearchCandidates>,
    _permit: Option<OwnedSemaphorePermit>,
}

impl FullTextSearch {
    #[must_use]
    pub fn new(index: Arc<SearchIndex>) -> Self {
        Self::with_budget(index, SearchBudget::default())
    }

    pub(crate) fn with_budget(index: Arc<SearchIndex>, budget: SearchBudget) -> Self {
        Self {
            index,
            permits: Arc::new(Semaphore::new(budget.concurrency)),
            budget,
        }
    }

    /// Begins a transaction holding every `entity` matching `query`, most relevant first, as
    /// its candidates; it has none without a query. The query is not blank.
    ///
    /// Unlike Kotlin, which keeps the first 1000 hits and finds nothing when the index fails,
    /// every hit is kept and a failure is an error.
    ///
    /// # Errors
    /// Fails when the search exceeds its budget, rather than keeping some of the hits.
    pub(crate) async fn begin(
        &self,
        db: &Database,
        entity: SearchEntity,
        query: Option<&str>,
    ) -> Result<Search, SearchIndexError> {
        let Some(query) = query else {
            return Ok(Search {
                transaction: db.begin().await?,
                candidates: None,
                _permit: None,
            });
        };
        let deadline = Instant::now() + self.budget.time_limit;
        let permit = time::timeout_at(deadline, Arc::clone(&self.permits).acquire_owned())
            .await
            .map_err(|_| SearchBudgetError::Busy)?
            .expect("The search semaphore is never closed");
        // A search that runs out of time drops its transaction here, with any candidates.
        let (transaction, candidates) =
            time::timeout_at(deadline, self.candidates(db, entity, query.to_owned()))
                .await
                .map_err(|_| SearchBudgetError::TimedOut)??;
        Ok(Search {
            transaction,
            candidates: Some(candidates),
            _permit: Some(permit),
        })
    }

    async fn candidates(
        &self,
        db: &Database,
        entity: SearchEntity,
        query: String,
    ) -> Result<(DbTransaction, SearchCandidates), SearchIndexError> {
        let index = Arc::clone(&self.index);
        let mut hits = task::spawn_blocking(move || index.hits(entity, &query)).await??;
        if hits.len() > self.budget.max_hits {
            return Err(SearchBudgetError::TooManyHits {
                hits: hits.len(),
                limit: self.budget.max_hits,
            }
            .into());
        }
        let mut transaction = db.begin().await?;
        let mut candidates = SearchCandidates::create(&mut transaction).await?;
        while !hits.is_empty() {
            let (rest, batch) = task::spawn_blocking(move || {
                let batch = hits.next_batch(BATCH);
                (hits, batch)
            })
            .await?;
            hits = rest;
            candidates.insert(&mut transaction, &batch?).await?;
        }
        candidates.analyze(&mut transaction).await?;
        Ok((transaction, candidates))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use komga_db::Database;
    use komga_search::{IndexUpdate, NgramOptions, SearchDocument, SearchIndex};
    use komga_test_support::TestDir;
    use komga_types::SearchEntity;

    use super::{FullTextSearch, Search, SearchBudget};
    use crate::catalog::search_index::{SearchBudgetError, SearchIndexError};

    /// As many as the pool has connections, so that holding this many searches uses each one.
    const CONNECTIONS: usize = 5;

    struct Fixture {
        db: Database,
        index: Arc<SearchIndex>,
        _directory: TestDir,
    }

    impl Fixture {
        /// An index of `documents` read lists named `Batman`.
        async fn new(documents: usize) -> Self {
            let directory = komga_test_support::tempdir().expect("Data directory");
            let db = Database::connect(&directory.path().join("database.sqlite"))
                .await
                .expect("Open database");
            let ngram = NgramOptions::new(3, 10, true).expect("Valid n-gram sizes");
            let index = SearchIndex::open(directory.path(), ngram).expect("Open index");
            let generation = index.new_generation().expect("New generation");
            let updates: Vec<IndexUpdate> = (0..documents)
                .map(|number| {
                    let mut document =
                        SearchDocument::new(SearchEntity::ReadList, format!("r{number}"));
                    document.add("name", "Batman");
                    IndexUpdate::Upsert(document)
                })
                .collect();
            generation.apply(&updates).expect("Index documents");
            index.switch(generation).expect("Switch generation");
            Self {
                db,
                index: Arc::new(index),
                _directory: directory,
            }
        }

        fn search(&self, budget: SearchBudget) -> FullTextSearch {
            FullTextSearch::with_budget(Arc::clone(&self.index), budget)
        }
    }

    fn budget() -> SearchBudget {
        SearchBudget {
            max_hits: 10,
            time_limit: Duration::from_secs(10),
            concurrency: CONNECTIONS,
        }
    }

    async fn begin(search: &FullTextSearch, db: &Database) -> Result<Search, SearchIndexError> {
        search
            .begin(db, SearchEntity::ReadList, Some("batman"))
            .await
    }

    fn over_budget(result: Result<Search, SearchIndexError>) -> SearchBudgetError {
        match result {
            Err(SearchIndexError::OverBudget(error)) => error,
            Err(error) => panic!("Expected a budget failure, got {error}"),
            Ok(_) => panic!("Expected a budget failure, got candidates"),
        }
    }

    #[tokio::test]
    async fn a_search_with_too_many_hits_fails_instead_of_keeping_some() {
        let f = Fixture::new(11).await;
        let search = f.search(budget());
        assert_eq!(
            over_budget(begin(&search, &f.db).await),
            SearchBudgetError::TooManyHits {
                hits: 11,
                limit: 10
            }
        );
        let roomier = f.search(SearchBudget {
            max_hits: 11,
            ..budget()
        });
        let within = begin(&roomier, &f.db).await.expect("A search at its limit");
        assert!(within.candidates.is_some());
    }

    #[tokio::test]
    async fn a_search_out_of_time_fails() {
        let f = Fixture::new(10).await;
        // With every connection taken the search cannot finish, so it must run out of time. A zero
        // limit would not do: a timer fires at the next millisecond tick, and a small search can end
        // before it.
        let mut held = Vec::new();
        for _ in 0..CONNECTIONS {
            held.push(f.db.begin().await.expect("Hold a connection"));
        }
        let hurried = f.search(SearchBudget {
            time_limit: Duration::from_millis(50),
            ..budget()
        });
        assert_eq!(
            over_budget(begin(&hurried, &f.db).await),
            SearchBudgetError::TimedOut
        );
    }

    #[tokio::test]
    async fn searches_beyond_the_concurrency_wait_then_fail_until_one_ends() {
        let f = Fixture::new(10).await;
        let search = f.search(SearchBudget {
            time_limit: Duration::from_millis(200),
            concurrency: 1,
            ..budget()
        });
        let first = begin(&search, &f.db).await.expect("The only slot is free");
        assert_eq!(
            over_budget(begin(&search, &f.db).await),
            SearchBudgetError::Busy
        );
        // Lists without a query hold no candidates and need no slot.
        search
            .begin(&f.db, SearchEntity::ReadList, None)
            .await
            .expect("A list without a query");
        drop(first);
        begin(&search, &f.db).await.expect("The slot was released");
    }

    #[tokio::test]
    async fn abandoned_searches_release_their_candidates() {
        let f = Fixture::new(10).await;
        let search = f.search(budget());
        for _ in 0..CONNECTIONS {
            // What cancelling a request does while its list is read.
            drop(begin(&search, &f.db).await.expect("Candidates"));
        }
        // A connection that kept its candidate table could not create it again, and holding this
        // many searches uses every connection.
        let mut held = Vec::new();
        for _ in 0..CONNECTIONS {
            held.push(begin(&search, &f.db).await.expect("No candidates remain"));
        }
    }
}
