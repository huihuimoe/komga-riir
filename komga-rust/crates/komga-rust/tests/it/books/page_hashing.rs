//! Page hashing: the first and last pages of the ready books of libraries that enable it.

use std::fs::File;
use std::path::Path;
use std::time::{Duration, SystemTime};

use komga_archive::digest::FileDigest;
use reqwest::{Method, StatusCode};
use serde_json::json;

use super::{JPEG, PNG, library_with_books};
use crate::libraries::{admin_json, route};
use crate::support::TestServer;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::{book, book_file, media_last_modified, pages};
use crate::support::tasks::{queue_kotlin_page_hashing, wait_for_scan_work};

/// `page.jpg` with an EXIF segment after its start marker.
fn jpeg_with_exif() -> Vec<u8> {
    let plain = cbz::fixture(JPEG);
    let exif = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\0";
    let length = u16::try_from(exif.len() + 2).expect("Short EXIF");
    [
        &plain[..2],
        &[0xff, 0xe1],
        &length.to_be_bytes(),
        exif,
        &plain[2..],
    ]
    .concat()
}

fn file_hash(bytes: &[u8]) -> String {
    let mut digest = FileDigest::new();
    digest.update(bytes);
    digest.finish()
}

/// Eight pages: two JPEGs that differ only in metadata, then six copies of a PNG.
fn eight_pages() -> Vec<u8> {
    let (jpeg, tagged, png) = (cbz::fixture(JPEG), jpeg_with_exif(), cbz::fixture(PNG));
    let names: Vec<String> = (3..=8).map(|number| format!("{number}.png")).collect();
    let mut members = vec![
        ("1.jpg", STORED, jpeg.as_slice()),
        ("2.jpg", STORED, &tagged),
    ];
    members.extend(
        names
            .iter()
            .map(|name| (name.as_str(), STORED, png.as_slice())),
    );
    cbz::zip(&members)
}

async fn page_hashes(pool: &sqlx::SqlitePool, book_id: &str) -> Vec<String> {
    pages(pool, book_id)
        .await
        .into_iter()
        .map(|page| page.4)
        .collect()
}

/// Restarts the server with a `HashBookPages` row Kotlin queued for the book, once it ran.
async fn restart_hashing_pages(server: TestServer, directory: &Path, book_id: &str) -> TestServer {
    server.stop().await;
    queue_kotlin_page_hashing(directory, book_id).await;
    let server = TestServer::start(directory).await;
    wait_for_scan_work(directory).await;
    server
}

#[tokio::test]
async fn enabling_page_hashing_hashes_the_first_and_last_pages_once() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) =
        library_with_books(directory.path(), &[("book.cbz", eight_pages())]).await;
    let (book_id, _) = book(&pool, "book").await;
    assert!(
        page_hashes(&pool, &book_id)
            .await
            .iter()
            .all(String::is_empty)
    );

    let server = restart_hashing_pages(server, directory.path(), &book_id).await;
    assert!(
        page_hashes(&pool, &book_id)
            .await
            .iter()
            .all(String::is_empty),
        "the library does not hash pages"
    );

    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"hashPages": true})),
        StatusCode::NO_CONTENT,
    )
    .await;
    wait_for_scan_work(directory.path()).await;
    let hashes = page_hashes(&pool, &book_id).await;
    let png = file_hash(&cbz::fixture(PNG));
    let jpeg = hashes[0].clone();
    assert_eq!(
        hashes,
        [&jpeg, &jpeg, &png, "", "", &png, &png, &png],
        "the JPEG metadata does not count"
    );
    assert_ne!(jpeg, file_hash(&cbz::fixture(JPEG)), "JPEGs are rewritten");

    let modified = media_last_modified(&pool, &book_id).await;
    let server = restart_hashing_pages(server, directory.path(), &book_id).await;
    assert_eq!(page_hashes(&pool, &book_id).await, hashes);
    assert_eq!(
        media_last_modified(&pool, &book_id).await,
        modified,
        "a repeated task changes nothing"
    );
    server.stop().await;
}

#[tokio::test]
async fn pages_of_an_unrecorded_file_version_are_hashed_after_a_scan_records_it() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) =
        library_with_books(directory.path(), &[("book.cbz", eight_pages())]).await;
    let (book_id, _) = book(&pool, "book").await;
    let time = book_file(&pool, &book_id).await.0;
    File::options()
        .write(true)
        .open(directory.path().join("comics/book.cbz"))
        .and_then(|file| file.set_modified(SystemTime::now() + Duration::from_secs(60)))
        .expect("Book time");

    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"hashPages": true})),
        StatusCode::NO_CONTENT,
    )
    .await;
    wait_for_scan_work(directory.path()).await;
    assert_ne!(
        book_file(&pool, &book_id).await.0,
        time,
        "page hashing queued a deep scan"
    );
    let hashes = page_hashes(&pool, &book_id).await;
    assert_eq!(hashes.iter().filter(|hash| !hash.is_empty()).count(), 6);
}
