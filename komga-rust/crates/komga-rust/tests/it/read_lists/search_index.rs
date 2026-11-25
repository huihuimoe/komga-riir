//! `SearchIndexLifecycle`: read lists and collections created, renamed, or deleted over HTTP are
//! found by their current name only, across restarts and rebuilds.

use std::fs;
use std::time::Duration;

use komga_types::SearchEntity;
use reqwest::{Method, StatusCode};
use serde_json::json;

use super::{create_list, fixture, send};
use crate::support::db::{inject_failure, remove_failure};
use crate::support::search::{hits, hits_in, open, pending, wait_for_generation, wait_for_index};
use crate::support::tasks::{queue_kotlin_index_upgrade, wait_for_tasks};
use crate::support::{TestServer, app_config, json};

const COLLECTIONS: &str = "/api/v1/collections";

async fn create_collection(server: &TestServer, name: &str, series: &str) -> String {
    let body = json!({"name": name, "seriesIds": [series]});
    let created = json(send(server, None, Method::POST, COLLECTIONS, Some(&body)).await).await;
    created["id"].as_str().expect("Collection ID").to_owned()
}

async fn rename(server: &TestServer, path: &str, name: &str) {
    let body = json!({"name": name});
    let response = send(server, None, Method::PATCH, path, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT, "{path}");
}

async fn delete(server: &TestServer, path: &str) {
    let response = send(server, None, Method::DELETE, path, None).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT, "{path}");
}

#[tokio::test]
async fn sets_are_indexed_by_their_current_name() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let kept = create_list(&f, "Batman Year One", &[&f.a1]).await;
    let removed = create_list(&f, "Dark Knight", &[&f.a2]).await;
    let renamed = create_collection(&f.server, "Flash", &f.series_a).await;
    let deleted = create_collection(&f.server, "Superman", &f.series_b).await;
    rename(&f.server, &format!("/api/v1/readlists/{kept}"), "Watchmen").await;
    delete(&f.server, &format!("/api/v1/readlists/{removed}")).await;
    rename(
        &f.server,
        &format!("{COLLECTIONS}/{renamed}"),
        "Green Arrow",
    )
    .await;
    delete(&f.server, &format!("{COLLECTIONS}/{deleted}")).await;
    // A change to the members alone leaves the name searchable.
    let body = json!({"bookIds": [&f.a2]});
    let path = format!("/api/v1/readlists/{kept}");
    let response = send(&f.server, None, Method::PATCH, &path, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    wait_for_index(&f.pool).await;
    f.server.stop().await;

    let index = open(directory.path());
    let found = |entity, query| hits_in(&index, entity, query);
    assert_eq!(found(SearchEntity::ReadList, "watchmen"), [kept.as_str()]);
    assert_eq!(found(SearchEntity::ReadList, "*:*"), [kept.as_str()]);
    for query in ["batman", "dark"] {
        assert!(found(SearchEntity::ReadList, query).is_empty(), "{query}");
    }
    assert_eq!(found(SearchEntity::Collection, "arrow"), [renamed.as_str()]);
    assert_eq!(found(SearchEntity::Collection, "*:*"), [renamed.as_str()]);
    for query in ["flash", "superman"] {
        assert!(found(SearchEntity::Collection, query).is_empty(), "{query}");
    }
    assert!(
        found(SearchEntity::Collection, "watchmen").is_empty(),
        "read lists are not collections"
    );
}

#[tokio::test]
async fn changes_committed_before_a_stop_are_indexed_at_the_next_start() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let mut f = fixture(directory.path()).await;
    wait_for_index(&f.pool).await;
    f.server.stop().await;
    // Committed, but stopped before the index is: as after a crash.
    let mut config = app_config(directory.path());
    config.lucene.commit_delay.value = Duration::from_secs(3600);
    f.server = TestServer::start_with(config).await;
    let list = create_list(&f, "Watchmen", &[&f.a1]).await;
    f.server.stop().await;
    assert!(pending(&f.pool).await > 0);
    assert!(hits(directory.path(), SearchEntity::ReadList, "watchmen").is_empty());

    let server = TestServer::start(directory.path()).await;
    wait_for_index(&f.pool).await;
    server.stop().await;

    assert_eq!(
        hits(directory.path(), SearchEntity::ReadList, "watchmen"),
        [list]
    );
}

#[tokio::test]
async fn a_failed_rebuild_keeps_the_current_generation_until_one_succeeds() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let directory = directory.path();
    let f = fixture(directory).await;
    let list = create_list(&f, "Batman", &[&f.a1]).await;
    wait_for_index(&f.pool).await;
    f.server.stop().await;
    fs::write(
        directory.join("application.yml"),
        "komga:\n  lucene:\n    index-analyzer:\n      min-gram: 4\n      max-gram: 4\n      preserve-original: false\n",
    )
    .expect("Configuration file");
    inject_failure(&f.pool, "BEFORE INSERT ON ERA_SEARCH_INDEX_PENDING").await;

    let server = TestServer::start(directory).await;
    wait_for_tasks(directory, |rows| {
        rows.iter()
            .any(|row| row.0 == "REBUILD_INDEX" && row.3.contains("eraFailure"))
    })
    .await;
    server.stop().await;
    // The 3-grams of the current generation still match.
    assert_eq!(
        hits(directory, SearchEntity::ReadList, "atm"),
        [list.as_str()]
    );

    remove_failure(&f.pool).await;
    let server = TestServer::start(directory).await;
    wait_for_generation(directory, 2).await;
    wait_for_index(&f.pool).await;
    server.stop().await;
    assert!(hits(directory, SearchEntity::ReadList, "atm").is_empty());
    assert_eq!(
        hits(directory, SearchEntity::ReadList, "atma"),
        [list.as_str()]
    );
}

#[tokio::test]
async fn kotlin_index_tasks_rebuild_the_index() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let directory = directory.path();
    let f = fixture(directory).await;
    let list = create_list(&f, "Batman", &[&f.a1]).await;
    wait_for_index(&f.pool).await;
    f.server.stop().await;
    queue_kotlin_index_upgrade(directory).await;

    let server = TestServer::start(directory).await;
    wait_for_generation(directory, 2).await;
    wait_for_tasks(directory, |rows| {
        rows.iter().all(|row| !row.0.ends_with("_INDEX"))
    })
    .await;
    wait_for_index(&f.pool).await;
    server.stop().await;

    assert_eq!(
        hits(directory, SearchEntity::ReadList, "batman"),
        [list.as_str()]
    );
}
