//! The search index of a server under test, which the API cannot query yet.

use std::fs;
use std::path::Path;
use std::time::Duration;

use komga_search::{NgramOptions, SearchIndex};
use komga_types::SearchEntity;
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::time;

/// How many entities have documents the server has yet to update.
pub async fn pending(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM ERA_SEARCH_INDEX_PENDING")
        .fetch_one(pool)
        .await
        .expect("Pending search index updates")
}

/// Waits until the server has applied every pending update to the index.
pub async fn wait_for_index(pool: &SqlitePool) {
    time::timeout(Duration::from_secs(10), async {
        loop {
            if pending(pool).await == 0 {
                return;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("The search index catches up");
}

/// Waits until the index of the server over `directory` has switched to `generation`.
pub async fn wait_for_generation(directory: &Path, generation: u64) {
    let manifest = directory.join("lucene/tantivy/generation.json");
    time::timeout(Duration::from_secs(10), async {
        loop {
            // The manifest is replaced by a rename, so it is never read half written.
            let current = fs::read(&manifest)
                .ok()
                .and_then(|manifest| serde_json::from_slice::<Value>(&manifest).ok())
                .and_then(|manifest| manifest["generation"].as_u64());
            if current == Some(generation) {
                return;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("The index switches generation");
}

/// Opens the search index of the stopped server over `directory` so tests running several queries
/// can reuse one reader.
pub fn open(directory: &Path) -> SearchIndex {
    SearchIndex::open(&directory.join("lucene"), NgramOptions::default()).expect("Open the index")
}

/// The IDs of the `entity` documents matching `query` in `index`, sorted.
pub fn hits_in(index: &SearchIndex, entity: SearchEntity, query: &str) -> Vec<String> {
    let mut hits = index.hits(entity, query).expect("Search");
    let mut hits = hits.next_batch(hits.len()).expect("Hit IDs");
    hits.sort();
    hits
}

/// The IDs of the `entity` documents matching `query` in the index of the stopped server over
/// `directory`, sorted.
pub fn hits(directory: &Path, entity: SearchEntity, query: &str) -> Vec<String> {
    hits_in(&open(directory), entity, query)
}
