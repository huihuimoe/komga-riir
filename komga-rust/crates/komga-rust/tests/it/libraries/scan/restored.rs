//! Moved books and series: the new ones a rescan finds take over the state of the soft-deleted
//! ones with the same file hashes.

use std::fs;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::scan;
use crate::libraries::{LIBRARIES, admin_json};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::{book, books, series};
use crate::support::tasks::{TaskRow, clear_after_scans, wait_for_scan_work};
use crate::support::{TestServer, db};

/// A readable book of one PNG page stored as `page`, so that each name gives another hash, and a
/// `ComicInfo.xml` titled after the page.
fn comic(page: &str) -> Vec<u8> {
    let png = cbz::fixture("komga-image/tests/fixtures/photo.png");
    let comic_info = format!("<ComicInfo><Title>Title of {page}</Title></ComicInfo>");
    cbz::zip(&[
        (page, STORED, &png),
        ("ComicInfo.xml", STORED, comic_info.as_bytes()),
    ])
}

/// A server whose library holds `files` under `comics`, analyzed and hashed by its first scan.
async fn library_with(directory: &Path, files: &[(&str, &str)]) -> (TestServer, Value, SqlitePool) {
    let server = TestServer::start_claimed(directory).await;
    let root = directory.join("comics");
    for (file, page) in files {
        let path = root.join(file);
        fs::create_dir_all(path.parent().expect("Book directory")).expect("Book directory");
        fs::write(&path, comic(page)).expect("Book file");
    }
    let body = json!({"name": "Comics", "root": root, "hashKoreader": true});
    let library = admin_json(
        &server,
        Method::POST,
        LIBRARIES,
        Some(&body),
        StatusCode::OK,
    )
    .await;
    let pool = db::main_pool(directory).await;
    clear_after_scans(&server, directory).await;
    (server, library, pool)
}

/// What a restoration hands over from a book: media status, page count and creation date, file
/// and KOReader hashes, and generated thumbnail IDs. A new analysis would give the same status,
/// pages, and hashes, but another media date and thumbnail.
type HandedOver = (String, i32, String, String, String, Option<String>);

async fn handed_over(pool: &SqlitePool, book_id: &str) -> HandedOver {
    sqlx::query_as(
        "SELECT m.STATUS, m.PAGE_COUNT, m.CREATED_DATE, b.FILE_HASH, b.FILE_HASH_KOREADER, \
         (SELECT GROUP_CONCAT(t.ID) FROM THUMBNAIL_BOOK t \
          WHERE t.BOOK_ID = b.ID AND t.TYPE = 'GENERATED') \
         FROM BOOK b JOIN MEDIA m ON m.BOOK_ID = b.ID WHERE b.ID = ?",
    )
    .bind(book_id)
    .fetch_one(pool)
    .await
    .expect("Stored book")
}

async fn rescan(server: &TestServer, directory: &Path, library: &Value) -> Vec<TaskRow> {
    scan(server, library, "").await;
    wait_for_scan_work(directory).await
}

async fn book_title(pool: &SqlitePool, book_id: &str) -> String {
    sqlx::query_scalar("SELECT TITLE FROM BOOK_METADATA WHERE BOOK_ID = ?")
        .bind(book_id)
        .fetch_one(pool)
        .await
        .expect("Book metadata")
}

#[tokio::test]
async fn a_book_moved_to_another_series_keeps_its_analysis_and_hashes() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let files = [
        ("Series A/a 1.cbz", "a.png"),
        ("Series A/a 2.cbz", "b.png"),
        ("Series B/b 1.cbz", "c.png"),
    ];
    let (server, library, pool) = library_with(directory.path(), &files).await;
    let (old_id, _) = book(&pool, "a 1").await;
    let before = handed_over(&pool, &old_id).await;
    assert_eq!(before.0, "READY");
    assert!(!before.4.is_empty(), "the KOReader hash is stored");
    assert!(before.5.is_some(), "the analysis generated a thumbnail");

    let root = directory.path().join("comics");
    fs::rename(root.join("Series A/a 1.cbz"), root.join("Series B/a 1.cbz")).expect("Move book");
    rescan(&server, directory.path(), &library).await;

    let (new_id, new_series) = book(&pool, "a 1").await;
    let series_b = series(&pool).await[1].0.clone();
    assert_eq!(new_series, series_b);
    assert_ne!(new_id, old_id);
    assert_eq!(handed_over(&pool, &new_id).await, before);
    assert!(
        books(&pool)
            .await
            .iter()
            .all(|row| row.0 != old_id && !row.3),
        "the restored book is deleted"
    );
    // The restored metadata takes the new file name as title, then its refresh reads the title
    // of `ComicInfo.xml` again.
    assert_eq!(book_title(&pool, &new_id).await, "Title of a.png");
    server.stop().await;
}

#[tokio::test]
async fn a_renamed_series_directory_keeps_its_books_and_takes_the_new_name() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let files = [("Old/1.cbz", "a.png"), ("Old/2.cbz", "b.png")];
    let (server, library, pool) = library_with(directory.path(), &files).await;
    let old_series = series(&pool).await;
    let mut before = Vec::new();
    for name in ["1", "2"] {
        let (id, _) = book(&pool, name).await;
        before.push(handed_over(&pool, &id).await);
    }

    let root = directory.path().join("comics");
    fs::rename(root.join("Old"), root.join("New")).expect("Rename series");
    rescan(&server, directory.path(), &library).await;

    let new_series = series(&pool).await;
    assert_eq!(
        new_series.len(),
        1,
        "the restored series is deleted: {new_series:?}"
    );
    assert_ne!(new_series[0].0, old_series[0].0);
    assert_eq!(new_series[0].1, "New");
    let title: String = sqlx::query_scalar("SELECT TITLE FROM SERIES_METADATA WHERE SERIES_ID = ?")
        .bind(&new_series[0].0)
        .fetch_one(&pool)
        .await
        .expect("Series metadata");
    assert_eq!(title, "New", "an unlocked title follows the directory");
    for (name, before) in ["1", "2"].into_iter().zip(before) {
        let (id, series_id) = book(&pool, name).await;
        assert_eq!(series_id, new_series[0].0);
        assert_eq!(handed_over(&pool, &id).await, before);
    }
    assert_eq!(books(&pool).await.len(), 2);
    server.stop().await;
}
