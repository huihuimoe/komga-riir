//! Imports killed while their file operation is open are completed or undone at the next start.

use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::process::Command;

use reqwest::header::COOKIE;
use reqwest::{Client, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use super::JPEG;
use crate::support::cbz::{self, STORED};
use crate::support::db::{self, WriteLock};
use crate::support::process::{Server, admin_session, create_library, free_port};
use crate::support::tasks::{wait_for_scan_work, wait_for_tasks};

const IMPORT_BOOK: &str = "org.gotson.komga.application.tasks.Task$ImportBook";

fn comic() -> Vec<u8> {
    cbz::zip(&[("1.jpg", STORED, &cbz::fixture(JPEG))])
}

/// A process serving a library with the series `Watchmen`, its session cookie, the main
/// database, the series ID, and a folder outside the library holding `files`.
async fn fixture(
    directory: &Path,
    port: u16,
    files: &[&str],
) -> (Server, String, SqlitePool, String) {
    let client = Client::new();
    let server = Server::start(directory, port, "first.log").await;
    let cookie = admin_session(&client, &server).await;
    let root = directory.join("comics");
    let series = root.join("Watchmen");
    fs::create_dir_all(&series).expect("Series directory");
    fs::write(series.join("Watchmen 1.cbz"), comic()).expect("Book file");
    create_library(&client, &server, &root).await;
    wait_for_scan_work(directory).await;
    let incoming = directory.join("incoming");
    fs::create_dir(&incoming).expect("Incoming folder");
    for file in files {
        fs::write(incoming.join(file), comic()).expect("Incoming file");
    }
    let main = db::main_pool(directory).await;
    let series_id = sqlx::query_scalar("SELECT ID FROM SERIES")
        .fetch_one(&main)
        .await
        .expect("Series");
    (server, cookie, main, series_id)
}

async fn import(server: &Server, cookie: &str, copy_mode: &str, source: &Path, series_id: &str) {
    let response = Client::new()
        .post(format!("{}/api/v1/books/import", server.base_url))
        .header(COOKIE, cookie)
        .json(&json!({"copyMode": copy_mode, "books": [{"sourceFile": source, "seriesId": series_id}]}))
        .send()
        .await
        .expect("Import request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

/// Waits until the imports, and the scan work they queue, ran.
async fn wait_for_imports(directory: &Path) {
    wait_for_tasks(directory, |rows| {
        rows.iter()
            .filter(|row| row.2 == IMPORT_BOOK)
            .all(|row| row.3.contains("\"eraFailed\""))
    })
    .await;
    wait_for_scan_work(directory).await;
}

async fn count(main: &SqlitePool, query: &'static str) -> i64 {
    sqlx::query_scalar(query)
        .fetch_one(main)
        .await
        .expect("Count rows")
}

#[cfg(unix)]
#[tokio::test]
async fn an_import_killed_after_publishing_its_book_is_undone_then_replayed_at_restart() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let port = free_port();
    let (server, cookie, main, series_id) =
        fixture(directory.path(), port, &["w2.cbz", "w2.jpg"]).await;
    let source = directory.path().join("incoming/w2.cbz");
    let target = directory.path().join("comics/Watchmen/w2.cbz");
    // Copying the sidecar opens this FIFO for writing, which blocks without a reader: the book is
    // published but not recorded.
    let blocked = directory.path().join("comics/Watchmen/w2.jpg");
    let status = Command::new("mkfifo")
        .arg(&blocked)
        .status()
        .expect("mkfifo");
    assert!(status.success(), "mkfifo {status}");

    import(&server, &cookie, "COPY", &source, &series_id).await;
    db::wait_for_rows(
        &main,
        "SELECT COUNT(*) FROM ERA_FILE_OPERATION WHERE OUTPUT_SIZE IS NOT NULL",
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while !target.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("The book is published");
    server.kill();
    fs::remove_file(&blocked).expect("Remove FIFO");

    // Had the published book not been undone, the replayed import would find its target taken.
    let server = Server::start(directory.path(), port, "second.log").await;
    wait_for_imports(directory.path()).await;
    let log = server.kill();
    assert!(target.exists() && source.exists(), "{log}");
    assert_eq!(
        count(&main, "SELECT COUNT(*) FROM ERA_FILE_OPERATION").await,
        0
    );
    assert_eq!(count(&main, "SELECT COUNT(*) FROM BOOK").await, 2, "{log}");
    let mut names: Vec<String> = fs::read_dir(directory.path().join("comics/Watchmen"))
        .expect("Series directory")
        .map(|entry| {
            entry
                .expect("Entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(names, ["Watchmen 1.cbz", "w2.cbz", "w2.jpg"]);
}

#[tokio::test]
async fn a_move_killed_after_its_commit_removes_its_source_at_restart() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let port = free_port();
    let (server, cookie, main, series_id) = fixture(directory.path(), port, &["w3.cbz"]).await;
    let source = directory.path().join("incoming/w3.cbz");
    let target = directory.path().join("comics/Watchmen/w3.cbz");

    // The main database lock holds the claimed import until the tasks database is locked too;
    // then the import commits and waits to queue its follow-ups before it removes the source.
    let main_lock = WriteLock::acquire(directory.path()).await;
    import(&server, &cookie, "MOVE", &source, &series_id).await;
    wait_for_tasks(directory.path(), |rows| {
        rows.iter()
            .any(|row| row.2 == IMPORT_BOOK && row.4.is_some())
    })
    .await;
    let tasks_lock = WriteLock::acquire_tasks(directory.path()).await;
    main_lock.release().await;
    db::wait_for_rows(
        &main,
        "SELECT COUNT(*) FROM ERA_FILE_OPERATION WHERE COMMITTED",
    )
    .await;
    server.kill();
    tasks_lock.release().await;
    assert!(source.exists() && target.exists());

    let server = Server::start(directory.path(), port, "second.log").await;
    wait_for_imports(directory.path()).await;
    let log = server.kill();
    assert!(!source.exists(), "{log}");
    assert!(target.exists());
    assert_eq!(
        count(&main, "SELECT COUNT(*) FROM ERA_FILE_OPERATION").await,
        0
    );
    assert_eq!(count(&main, "SELECT COUNT(*) FROM BOOK").await, 2);
    assert_eq!(
        count(&main, "SELECT COUNT(*) FROM MEDIA m JOIN BOOK b ON b.ID = m.BOOK_ID WHERE b.NAME = 'w3' AND m.STATUS = 'READY'").await,
        1,
        "{log}"
    );
}
