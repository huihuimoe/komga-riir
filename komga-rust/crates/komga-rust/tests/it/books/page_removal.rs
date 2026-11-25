//! Pages removed from book files by hash (`BookPageEditor.removeHashedPages`).

use std::fs;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::library_with_books;
use super::reading::fetch;
use super::thumbnails::cover;
use crate::libraries::{admin_json, route};
use crate::support::TestServer;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::{book, files, media, pages};
use crate::support::tasks::{TaskRow, wait_for_scan_work, wait_for_tasks};

const PAGE_HASHES: &str = "/api/v1/page-hashes";
const COMIC_INFO: &[u8] = b"<ComicInfo><Title>Kept</Title></ComicInfo>";

/// A library whose books have their pages hashed: `a` is the cover, a photo and `ComicInfo.xml`,
/// `b` the cover twice, `c` the photo only; `d` is a RAR book.
async fn hashed_library(directory: &Path) -> (TestServer, Value, SqlitePool) {
    let (cover, photo) = (cover(), cbz::fixture(super::PNG));
    let books = [
        (
            "a.cbz",
            cbz::zip(&[
                ("1.png", STORED, &cover),
                ("2.png", STORED, &photo),
                ("ComicInfo.xml", STORED, COMIC_INFO),
            ]),
        ),
        (
            "b.cbz",
            cbz::zip(&[("1.png", STORED, &cover), ("2.png", STORED, &cover)]),
        ),
        ("c.cbz", cbz::zip(&[("1.png", STORED, &photo)])),
        (
            "d.cbr",
            cbz::fixture("komga-archive/tests/fixtures/rar/rar4-solid.rar"),
        ),
    ];
    let (server, library, pool) = library_with_books(directory, &books).await;
    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"hashPages": true})),
        StatusCode::NO_CONTENT,
    )
    .await;
    wait_for_scan_work(directory).await;
    (server, library, pool)
}

/// The hash of page `number` (from 1) of the book named `name`.
async fn page_hash(pool: &SqlitePool, name: &str, number: usize) -> String {
    let (id, _) = book(pool, name).await;
    pages(pool, &id).await[number - 1].4.clone()
}

/// Marks `hash` known with `action`.
async fn mark(server: &TestServer, hash: &str, action: &str) {
    let body = json!({"hash": hash, "action": action});
    let request = server.admin(Method::PUT, PAGE_HASHES).json(&body);
    assert_eq!(fetch(request).await.status, StatusCode::ACCEPTED);
}

async fn delete_count(server: &TestServer, hash: &str) -> i64 {
    let known = fetch(server.admin(Method::GET, PAGE_HASHES)).await;
    let known: Value = serde_json::from_slice(&known.body).expect("Known hashes");
    let known = known["content"].as_array().expect("Page content");
    let item = known
        .iter()
        .find(|item| item["hash"] == hash)
        .expect("Known hash");
    item["deleteCount"].as_i64().expect("Delete count")
}

async fn post(server: &TestServer, path: &str, body: Option<&Value>) {
    let mut request = server.admin(Method::POST, &format!("{PAGE_HASHES}/{path}"));
    if let Some(body) = body {
        request = request.json(body);
    }
    assert_eq!(fetch(request).await.status, StatusCode::ACCEPTED, "{path}");
}

async fn thumbnail(server: &TestServer, book_id: &str) -> Vec<u8> {
    let path = format!("/api/v1/books/{book_id}/thumbnail");
    fetch(server.admin(Method::GET, &path)).await.body
}

/// Waits until every queued task has run or failed; the failed ones stay queued.
async fn settle(directory: &Path) -> Vec<TaskRow> {
    let failed = |rows: &[TaskRow]| rows.iter().all(|row| row.3.contains("\"eraFailed\""));
    wait_for_tasks(directory, failed).await
}

/// No file operation is left over beside the books.
fn assert_no_leftovers(root: &Path) {
    let names: Vec<_> = fs::read_dir(root)
        .expect("Library root")
        .map(|entry| {
            entry
                .expect("Entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with(".komga-"))
        .collect();
    assert_eq!(names, Vec::<String>::new());
}

#[tokio::test]
async fn deleting_all_matches_rewrites_each_book_without_them_keeping_the_rest() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = hashed_library(directory.path()).await;
    let root = directory.path().join("comics");
    let (a, _) = book(&pool, "a").await;
    let (cover_hash, photo_hash) = (
        page_hash(&pool, "a", 1).await,
        page_hash(&pool, "a", 2).await,
    );
    let b_file = fs::read(root.join("b.cbz")).expect("b");
    let a_thumbnail = thumbnail(&server, &a).await;
    mark(&server, &cover_hash, "DELETE_MANUAL").await;

    post(&server, &format!("{cover_hash}/delete-all"), None).await;
    let failed = settle(directory.path()).await;
    // `b` would be left without pages: its file fails verification and stays as it was.
    let (b, _) = book(&pool, "b").await;
    let failed: Vec<_> = failed.iter().map(|row| row.0.as_str()).collect();
    assert_eq!(failed, [format!("REMOVE_HASHED_PAGES_{b}")]);

    // `a` keeps its photo, with its hash, and its other member; its thumbnail is the photo now.
    let a_pages = pages(&pool, &a).await;
    let a_pages: Vec<_> = a_pages
        .iter()
        .map(|page| (page.0.as_str(), page.4.as_str()))
        .collect();
    assert_eq!(a_pages, [("2.png", photo_hash.as_str())]);
    assert_eq!(media(&pool, &a).await.0, "READY");
    assert_eq!(files(&pool, &a).await[0].0, "ComicInfo.xml");
    let members = cbz::entries(fs::read(root.join("a.cbz")).expect("a"));
    let names: Vec<_> = members.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["2.png", "ComicInfo.xml"]);
    assert_eq!(members[1].1, COMIC_INFO);
    assert_ne!(thumbnail(&server, &a).await, a_thumbnail);
    assert_eq!(fs::read(root.join("b.cbz")).expect("b"), b_file);
    assert_eq!(delete_count(&server, &cover_hash).await, 1);
    assert_no_leftovers(&root);

    // A replayed removal no longer matches the pages and changes nothing.
    let replayed = json!({
        "bookId": a, "url": "/a.cbz", "pageNumber": 1, "fileName": "1.png", "fileSize": 1,
        "mediaType": "image/png",
    });
    let a_file = fs::read(root.join("a.cbz")).expect("a");
    post(
        &server,
        &format!("{cover_hash}/delete-match"),
        Some(&replayed),
    )
    .await;
    settle(directory.path()).await;
    assert_eq!(fs::read(root.join("a.cbz")).expect("a"), a_file);
    assert_eq!(delete_count(&server, &cover_hash).await, 1);
    server.stop().await;
}

#[tokio::test]
async fn automatic_deletion_follows_scans_and_spares_unsupported_or_mismatched_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = hashed_library(directory.path()).await;
    let root = directory.path().join("comics");
    let (a, _) = book(&pool, "a").await;
    let photo_hash = page_hash(&pool, "a", 2).await;
    // The single page of `c` is the photo: its removal must not apply to `a`'s second page.
    let (c, _) = book(&pool, "c").await;
    let a_photo = json!({
        "bookId": a, "url": "/a.cbz", "pageNumber": 1, "fileName": "2.png", "fileSize": 1,
        "mediaType": "image/png",
    });
    post(
        &server,
        &format!("{photo_hash}/delete-match"),
        Some(&a_photo),
    )
    .await;
    settle(directory.path()).await;
    assert_eq!(
        pages(&pool, &a).await.len(),
        2,
        "the page number does not match"
    );

    // Only ZIP books are rewritten.
    let (d, _) = book(&pool, "d").await;
    let d_page = &pages(&pool, &d).await[0];
    let d_match = json!({
        "bookId": d, "url": "/d.cbr", "pageNumber": 1, "fileName": d_page.0,
        "fileSize": 1, "mediaType": d_page.1,
    });
    let d_file = fs::read(root.join("d.cbr")).expect("d");
    post(
        &server,
        &format!("{}/delete-match", d_page.4),
        Some(&d_match),
    )
    .await;
    let failed = settle(directory.path()).await;
    assert!(
        failed[0].3.contains("cannot be converted. Must be one of"),
        "{failed:?}"
    );
    assert_eq!(fs::read(root.join("d.cbr")).expect("d"), d_file);

    let c_file = fs::read(root.join("c.cbz")).expect("c");
    mark(&server, &photo_hash, "DELETE_AUTO").await;
    let scan = server.admin(Method::POST, &format!("{}/scan", route(&library)));
    assert_eq!(fetch(scan).await.status, StatusCode::ACCEPTED);
    settle(directory.path()).await;

    let a_pages = pages(&pool, &a).await;
    assert_eq!(
        a_pages
            .iter()
            .map(|page| page.0.as_str())
            .collect::<Vec<_>>(),
        ["1.png"]
    );
    assert_eq!(
        fs::read(root.join("c.cbz")).expect("c"),
        c_file,
        "no book is left empty"
    );
    assert_eq!(pages(&pool, &c).await.len(), 1);
    assert_eq!(delete_count(&server, &photo_hash).await, 1);
    assert_no_leftovers(&root);
    server.stop().await;
}

/// `(TYPE, KEY, VALUE)` of each history property of the book, by key.
async fn history(pool: &SqlitePool, book_id: &str) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT E.TYPE, P.KEY, P.VALUE FROM HISTORICAL_EVENT E \
         JOIN HISTORICAL_EVENT_PROPERTIES P ON P.ID = E.ID WHERE E.BOOK_ID = ? ORDER BY P.KEY",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .expect("History")
}

#[tokio::test]
async fn a_removal_is_recorded_once_and_moves_reading_progress() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = hashed_library(directory.path()).await;
    let root = directory.path().join("comics");
    let (a, series) = book(&pool, "a").await;
    let cover_hash = page_hash(&pool, "a", 1).await;
    let progress = server
        .admin(Method::PATCH, &format!("/api/v1/books/{a}/read-progress"))
        .json(&json!({"page": 2}));
    assert_eq!(fetch(progress).await.status, StatusCode::NO_CONTENT);
    let cover = json!({
        "bookId": a, "url": "/a.cbz", "pageNumber": 1, "fileName": "1.png", "fileSize": 6278,
        "mediaType": "image/png",
    });

    for _ in 0..2 {
        post(&server, &format!("{cover_hash}/delete-match"), Some(&cover)).await;
        settle(directory.path()).await;
    }

    let path = root.join("a.cbz").to_string_lossy().into_owned();
    let expected = [
        ("name", path.as_str()),
        ("page file hash", cover_hash.as_str()),
        ("page file name", "1.png"),
        ("page file size", "6278"),
        ("page media type", "image/png"),
        ("page number", "1"),
    ]
    .map(|(key, value)| {
        (
            "DuplicatePageDeleted".to_owned(),
            key.to_owned(),
            value.to_owned(),
        )
    });
    assert_eq!(history(&pool, &a).await, expected);
    let series_id: Option<String> =
        sqlx::query_scalar("SELECT SERIES_ID FROM HISTORICAL_EVENT WHERE BOOK_ID = ?")
            .bind(&a)
            .fetch_one(&pool)
            .await
            .expect("Series");
    assert_eq!(series_id, Some(series));
    // The page read no longer exists: the progress restarts on the first page.
    let book = fetch(server.admin(Method::GET, &format!("/api/v1/books/{a}"))).await;
    let book: Value = serde_json::from_slice(&book.body).expect("Book");
    assert_eq!(book["readProgress"]["page"], 1);
    assert_eq!(book["media"]["pagesCount"], 1);
    server.stop().await;
}
