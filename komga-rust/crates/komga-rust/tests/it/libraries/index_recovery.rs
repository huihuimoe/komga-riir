//! Search index recovery after the server process is killed.

use std::fs;
use std::path::Path;
use std::time::Duration;

use komga_types::SearchEntity;
use reqwest::Client;
use sqlx::SqlitePool;
use tokio::time;

use crate::support::cbz::{self, STORED};
use crate::support::db::{self, WriteLock};
use crate::support::process::{Server, admin_session, create_library, free_port};
use crate::support::search::{hits, pending, wait_for_generation, wait_for_index};
use crate::support::tasks::wait_for_scan_work;

/// Starts a process over a library with the series `Watchmen`, scanned, and returns it with the
/// main database and the series ID.
async fn fixture(directory: &Path, port: u16) -> (Server, SqlitePool, String) {
    let client = Client::new();
    let server = Server::start(directory, port, "first.log").await;
    admin_session(&client, &server).await;
    let root = directory.join("comics");
    fs::create_dir_all(root.join("Watchmen")).expect("Series directory");
    let book = cbz::zip(&[("1.png", STORED, b"png")]);
    fs::write(root.join("Watchmen/Watchmen 1.cbz"), book).expect("Book file");
    create_library(&client, &server, &root).await;
    wait_for_scan_work(directory).await;
    let main = db::main_pool(directory).await;
    let series = sqlx::query_scalar("SELECT ID FROM SERIES")
        .fetch_one(&main)
        .await
        .expect("Series");
    (server, main, series)
}

fn configure(directory: &Path, lucene: &str) {
    let configuration = format!("komga:\n  lucene:\n{lucene}");
    fs::write(directory.join("application.yml"), configuration).expect("Configuration file");
}

#[tokio::test]
async fn changes_committed_before_a_kill_are_indexed_at_restart() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let directory = directory.path();
    let port = free_port();
    // The index would wait an hour to apply the scan.
    configure(directory, "    commit-delay: 3600000\n");
    let (server, main, series) = fixture(directory, port).await;
    server.kill();
    assert!(pending(&main).await > 0);
    assert!(hits(directory, SearchEntity::Series, "watchmen").is_empty());

    let server = Server::start(directory, port, "second.log").await;
    wait_for_index(&main).await;
    server.kill();
    assert_eq!(hits(directory, SearchEntity::Series, "watchmen"), [series]);
}

#[tokio::test]
async fn a_rebuild_killed_midway_keeps_the_current_generation_and_runs_again() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let directory = directory.path();
    let port = free_port();
    let (server, main, series) = fixture(directory, port).await;
    wait_for_index(&main).await;
    server.kill();

    // New n-grams call for a rebuild at start; the main database lock holds it once it created
    // the next generation.
    configure(
        directory,
        "    index-analyzer:\n      min-gram: 4\n      max-gram: 4\n      preserve-original: false\n",
    );
    let lock = WriteLock::acquire(directory).await;
    let server = Server::start(directory, port, "second.log").await;
    let next = directory.join("lucene/tantivy/2");
    time::timeout(Duration::from_secs(10), async {
        while !next.exists() {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("The rebuild creates the next generation");
    server.kill();
    lock.release().await;
    // The 3-grams of the current generation still match.
    assert_eq!(
        hits(directory, SearchEntity::Series, "atc"),
        [series.as_str()]
    );

    let server = Server::start(directory, port, "third.log").await;
    wait_for_generation(directory, 2).await;
    wait_for_index(&main).await;
    server.kill();
    assert!(hits(directory, SearchEntity::Series, "atc").is_empty());
    assert_eq!(hits(directory, SearchEntity::Series, "atch"), [series]);
}
