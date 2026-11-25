//! Analyses of a file version the book row does not record are dropped; the library is scanned
//! instead when the file is the cause.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use reqwest::{Method, StatusCode};
use rustix::fs::{Mode, OFlags};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tokio::time;

use super::{PNG, mixed_book};
use crate::books::{analyze, library_with_books};
use crate::libraries::{admin_json, route};
use crate::support::TestServer;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::{book, book_file, media};
use crate::support::tasks::{wait_for_scan_work, wait_for_tasks};

fn ready() -> (String, Option<String>, Option<String>, i32) {
    (
        "READY".to_owned(),
        Some("application/zip".to_owned()),
        Some("ERR_1007 [bad.png]".to_owned()),
        3,
    )
}

/// The stale result would be `unsupported()`; instead the stored media keeps its pages and only
/// the scan marked it outdated.
fn outdated() -> (String, Option<String>, Option<String>, i32) {
    let (_, media_type, comment, page_count) = ready();
    ("OUTDATED".to_owned(), media_type, comment, page_count)
}

/// The FIFO is read as an empty, unsupported file once the analysis gets through.
fn unsupported() -> (String, Option<String>, Option<String>, i32) {
    (
        "UNSUPPORTED".to_owned(),
        Some("application/octet-stream".to_owned()),
        Some("ERR_1001".to_owned()),
        0,
    )
}

async fn deep_scan(server: &TestServer, library: &Value) {
    admin_json(
        server,
        Method::POST,
        &format!("{}/scan?deep=true", route(library)),
        None,
        StatusCode::ACCEPTED,
    )
    .await;
}

async fn wait_for_recorded_time(pool: &SqlitePool, book_id: &str, time: impl Fn(&str) -> bool) {
    time::timeout(Duration::from_secs(10), async {
        while !time(&book_file(pool, book_id).await.0) {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("A scan records the expected file time");
}

/// The book file as a FIFO. Dropping it releases threads still blocked opening it, so a failed
/// assertion fails the test instead of leaving the runtime waiting on those threads.
struct Fifo {
    path: PathBuf,
}

impl Drop for Fifo {
    fn drop(&mut self) {
        // While both ends are held, pending and new opens complete; afterwards none can block.
        let read = rustix::fs::open(&self.path, OFlags::RDONLY | OFlags::NONBLOCK, Mode::empty());
        let write = rustix::fs::open(&self.path, OFlags::WRONLY | OFlags::NONBLOCK, Mode::empty());
        let _ = fs::remove_file(&self.path);
        drop((read, write));
    }
}

/// A ready book whose file was replaced by a FIFO and deep scanned: the scan records the FIFO,
/// marks the media outdated, and the analysis it queues blocks opening the FIFO until the test
/// opens the write end, so the test can change the file or the database meanwhile.
async fn analysis_behind_a_fifo(directory: &Path) -> (TestServer, Value, SqlitePool, String, Fifo) {
    let (server, library, pool) =
        library_with_books(directory, &[("book.cbz", mixed_book())]).await;
    let (book_id, _) = book(&pool, "book").await;
    assert_eq!(media(&pool, &book_id).await, ready());
    // Hashing would also block opening the FIFO.
    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"hashFiles": false})),
        StatusCode::NO_CONTENT,
    )
    .await;
    let path = directory.join("comics/book.cbz");
    fs::remove_file(&path).expect("Remove book");
    let status = Command::new("mkfifo").arg(&path).status().expect("mkfifo");
    assert!(status.success(), "mkfifo {status}");
    let fifo = Fifo { path };
    deep_scan(&server, &library).await;
    time::timeout(Duration::from_secs(10), async {
        while media(&pool, &book_id).await != outdated() {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("The scan marks the media outdated");
    (server, library, pool, book_id, fifo)
}

/// Waits for an analysis to open the FIFO for reading, and returns the write end.
async fn writer(path: &Path) -> File {
    let path = path.to_owned();
    let open = tokio::task::spawn_blocking(move || OpenOptions::new().write(true).open(path));
    time::timeout(Duration::from_secs(10), open)
        .await
        .expect("An analysis opens the book within ten seconds")
        .expect("Open worker")
        .expect("FIFO write end")
}

#[tokio::test]
async fn a_file_replaced_since_the_scan_is_scanned_before_it_is_analyzed() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("book.cbz", mixed_book())]).await;
    let (book_id, _) = book(&pool, "book").await;
    let replacement = cbz::zip(&[("only.png", STORED, &cbz::fixture(PNG))]);
    fs::write(directory.path().join("comics/book.cbz"), &replacement).expect("Replace book");

    analyze(&server, &book_id).await;
    wait_for_scan_work(directory.path()).await;
    let size = i64::try_from(replacement.len()).expect("Small book");
    assert_eq!(book_file(&pool, &book_id).await.1, size);
    assert_eq!(
        media(&pool, &book_id).await,
        (
            "READY".to_owned(),
            Some("application/zip".to_owned()),
            None,
            1
        )
    );
    server.stop().await;
}

#[tokio::test]
async fn a_file_changed_while_it_is_read_is_scanned_instead_of_stored() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool, book_id, fifo) = analysis_behind_a_fifo(directory.path()).await;
    let (fifo_time, _) = book_file(&pool, &book_id).await;

    let file = writer(&fifo.path).await;
    file.set_modified(SystemTime::now() + Duration::from_secs(60))
        .expect("Change the file time");
    drop(file);
    // The dropped analysis queues a deep scan; it records the new time and queues the analysis
    // again, which waits on the FIFO.
    wait_for_recorded_time(&pool, &book_id, |time| time != fifo_time).await;
    assert_eq!(media(&pool, &book_id).await, outdated());
    drop(writer(&fifo.path).await);
    wait_for_scan_work(directory.path()).await;
    assert_eq!(media(&pool, &book_id).await, unsupported());
    server.stop().await;
}

#[tokio::test]
async fn an_analysis_overtaken_by_a_scan_is_dropped() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool, book_id, fifo) = analysis_behind_a_fifo(directory.path()).await;
    let (fifo_time, _) = book_file(&pool, &book_id).await;
    // A second worker scans while the analysis waits.
    admin_json(
        &server,
        Method::PATCH,
        "/api/v1/settings",
        Some(&json!({"taskPoolSize": 2})),
        StatusCode::NO_CONTENT,
    )
    .await;

    let file = writer(&fifo.path).await;
    let modified = fs::metadata(&fifo.path)
        .and_then(|metadata| metadata.modified())
        .expect("FIFO time");
    file.set_modified(SystemTime::now() + Duration::from_secs(60))
        .expect("Change the file time");
    deep_scan(&server, &library).await;
    wait_for_tasks(directory.path(), |rows| {
        !rows.iter().any(|row| row.2.ends_with("$ScanLibrary"))
    })
    .await;
    // Restoring the time leaves the file as the analysis first saw it; only the book row changed.
    file.set_modified(modified).expect("Restore the file time");
    drop(file);
    // The analysis finds the file no longer matches the row and queues a deep scan, which records
    // the restored time and queues an analysis that waits on the FIFO.
    wait_for_recorded_time(&pool, &book_id, |time| time == fifo_time).await;
    assert_eq!(media(&pool, &book_id).await, outdated());
    // The scan also resubmitted the running analysis; depending on whether its rerun starts
    // before or after the second scan records the time, one or two analyses read the FIFO.
    loop {
        tokio::select! {
            _ = wait_for_scan_work(directory.path()) => break,
            file = writer(&fifo.path) => drop(file),
        }
    }
    assert_eq!(media(&pool, &book_id).await, unsupported());
    server.stop().await;
}
