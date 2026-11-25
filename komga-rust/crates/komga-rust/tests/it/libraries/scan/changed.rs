//! Book files replaced on disk: the scan keeps the book and, unless its file hash is unchanged,
//! marks its analysis outdated, which queues a new analysis.

use std::fs::{self, File};
use std::path::Path;
use std::time::{Duration, SystemTime};

use reqwest::{Method, StatusCode};
use serde_json::json;

use super::{BOOK_BYTES_HASH, scan, scanned_library, touch};
use crate::libraries::{LIBRARIES, admin_json, route};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::{
    book, book_file, book_hashes, media_last_modified, scanned_books,
};
use crate::support::tasks::{clear_after_scans, wait_for_scan_work};
use crate::support::{TestServer, db};

#[tokio::test]
async fn a_replaced_book_keeps_its_identity_and_is_analyzed_again() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool, _) = scanned_library(directory.path()).await;
    clear_after_scans(&server, directory.path()).await;
    let stored = scanned_books(&pool).await;
    let replaced = stored
        .iter()
        .position(|row| row.1 == "a 2")
        .expect("Stored book");
    let time = book_file(&pool, &stored[replaced].0).await.0;
    let path = directory.path().join("comics/Series A/a 2.cbz");
    let replacement = cbz::zip(&[(
        "page.png",
        STORED,
        &cbz::fixture("komga-image/tests/fixtures/photo.png"),
    )]);
    fs::write(&path, &replacement).expect("Replace book");
    File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_modified(SystemTime::now() + Duration::from_secs(60)))
        .expect("Book time");

    // Rewriting a file leaves its directory time alone, so only a deep scan looks at its books.
    scan(&server, &library, "").await;
    wait_for_scan_work(directory.path()).await;
    assert_eq!(scanned_books(&pool).await, stored);

    clear_after_scans(&server, directory.path()).await;
    scan(&server, &library, "?deep=true").await;
    wait_for_scan_work(directory.path()).await;
    let mut expected = stored.clone();
    expected[replaced].4 = i64::try_from(replacement.len()).expect("Small book");
    expected[replaced].7 = "READY".to_owned();
    assert_eq!(scanned_books(&pool).await, expected);
    assert_ne!(
        book_file(&pool, &stored[replaced].0).await.0,
        time,
        "the new file time is stored, so later scans see the book unchanged"
    );

    // The new state is stored, so the next deep scan changes nothing.
    scan(&server, &library, "?deep=true").await;
    wait_for_scan_work(directory.path()).await;
    assert_eq!(scanned_books(&pool).await, expected);
    server.stop().await;
}

/// Rewrites `path` with `bytes` and moves its time a minute ahead, as a later write would.
fn rewrite(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("Rewrite book");
    File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now() + Duration::from_secs(60)))
        .expect("Book time");
}

#[tokio::test]
async fn a_rewritten_book_of_equal_size_keeps_its_analysis_only_with_an_equal_hash() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool, _) = scanned_library(directory.path()).await;
    clear_after_scans(&server, directory.path()).await;
    let (book, _) = book(&pool, "a 2").await;
    let path = directory.path().join("comics/Series A/a 2.cbz");
    let analyzed = media_last_modified(&pool, &book).await;

    // Same bytes, new time: the hash Kotlin compares is equal, so only the file state changes.
    let time = book_file(&pool, &book).await.0;
    rewrite(&path, b"book bytes");
    scan(&server, &library, "?deep=true").await;
    wait_for_scan_work(directory.path()).await;
    assert_ne!(book_file(&pool, &book).await.0, time);
    assert_eq!(media_last_modified(&pool, &book).await, analyzed);
    assert_eq!(
        book_hashes(&pool, &book).await,
        (BOOK_BYTES_HASH.to_owned(), String::new())
    );

    // Same size, other bytes: the scan stores the new hash and the book is analyzed again.
    rewrite(&path, b"book BYTES");
    scan(&server, &library, "?deep=true").await;
    wait_for_scan_work(directory.path()).await;
    assert_ne!(media_last_modified(&pool, &book).await, analyzed);
    assert_eq!(
        book_hashes(&pool, &book).await,
        ("160bf34989f16b59dc2fdda42b2b005e".to_owned(), String::new())
    );
    server.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn an_unreadable_rewritten_book_does_not_block_the_scan() {
    use std::os::unix::fs::PermissionsExt;

    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool, _) = scanned_library(directory.path()).await;
    clear_after_scans(&server, directory.path()).await;
    let (book, _) = book(&pool, "a 2").await;
    let path = directory.path().join("comics/Series A/a 2.cbz");
    let time = book_file(&pool, &book).await.0;

    // Same size, so the scan needs a hash it cannot read.
    rewrite(&path, b"book BYTES");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("Lock book");
    scan(&server, &library, "?deep=true").await;
    wait_for_scan_work(directory.path()).await;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("Unlock book");
    assert_ne!(book_file(&pool, &book).await.0, time);
    assert_eq!(
        book_hashes(&pool, &book).await,
        (String::new(), String::new())
    );
    server.stop().await;
}

#[tokio::test]
async fn hashing_a_book_rewritten_since_the_scan_records_it_first() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let root = directory.path().join("comics");
    let path = root.join("Series A/a 1.cbz");
    touch(&path);
    let body = json!({"name": "Comics", "root": root, "hashFiles": false});
    let library = admin_json(
        &server,
        Method::POST,
        LIBRARIES,
        Some(&body),
        StatusCode::OK,
    )
    .await;
    let pool = db::main_pool(directory.path()).await;
    wait_for_scan_work(directory.path()).await;
    let (book, _) = book(&pool, "a 1").await;
    assert_eq!(
        book_hashes(&pool, &book).await,
        (String::new(), String::new())
    );

    // The hash of the new bytes would describe no recorded version: the hashing task rescans
    // deeply, and the rescan queues the hash of the version it records.
    rewrite(&path, b"book BYTES");
    let update = json!({"hashFiles": true});
    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&update),
        StatusCode::NO_CONTENT,
    )
    .await;
    wait_for_scan_work(directory.path()).await;
    assert_eq!(
        book_hashes(&pool, &book).await,
        ("160bf34989f16b59dc2fdda42b2b005e".to_owned(), String::new())
    );
    assert_eq!(scanned_books(&pool).await[0].7, "UNSUPPORTED");
    server.stop().await;
}
