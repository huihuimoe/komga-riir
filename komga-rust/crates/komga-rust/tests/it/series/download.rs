//! `GET /api/v1/series/{id}/file`.

use std::fs;

use reqwest::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde_json::json;

use crate::books::{PNG, library_with_books};
use crate::read_lists::{create_user, rate, send};
use crate::support::cbz::{self, STORED, entries};
use crate::support::db::content::book;

#[tokio::test]
async fn a_series_archive_holds_each_book_file_by_name() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let png = cbz::fixture(PNG);
    let a = cbz::zip(&[("1.png", STORED, &png)]);
    let b = cbz::zip(&[("1.png", STORED, &png), ("2.png", STORED, &png)]);
    let books = [
        ("Series/a.cbz", a.clone()),
        ("Series/b.cbz", b.clone()),
        ("Series/c.cbz", a.clone()),
    ];
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let (_, series_id) = book(&pool, "a").await;
    fs::remove_file(directory.path().join("comics/Series/c.cbz")).expect("Remove a book file");
    let path = format!("/api/v1/series/{series_id}/file");

    let response = send(&server, None, Method::GET, &path, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/zip");
    assert_eq!(
        response.headers()[CONTENT_DISPOSITION],
        "attachment; filename=\"=?UTF-8?Q?Series.zip?=\"; filename*=UTF-8''Series.zip"
    );
    let mut found = entries(response.bytes().await.expect("Archive").to_vec());
    found.sort();
    assert!(
        found == [("a.cbz".to_owned(), a), ("b.cbz".to_owned(), b)],
        "every book with a file"
    );
    server.stop().await;
}

#[tokio::test]
async fn series_downloads_need_file_download_and_a_visible_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let png = cbz::fixture(PNG);
    let books = [("Series/a.cbz", cbz::zip(&[("1.png", STORED, &png)]))];
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let (_, series_id) = book(&pool, "a").await;
    let path = format!("/api/v1/series/{series_id}/file");
    let missing = "/api/v1/series/missing/file";

    create_user(&server, "reader@example.com", json!({"roles": []})).await;
    for path in [path.as_str(), missing] {
        let response = send(&server, Some("reader@example.com"), Method::GET, path, None).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    rate(&server, &series_id, 18).await;
    let sixteen = json!({"roles": ["FILE_DOWNLOAD"],
        "ageRestriction": {"age": 16, "restriction": "ALLOW_ONLY"}});
    create_user(&server, "teen@example.com", sixteen).await;
    let response = send(&server, Some("teen@example.com"), Method::GET, &path, None).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = send(
        &server,
        Some("teen@example.com"),
        Method::GET,
        missing,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = send(&server, None, Method::GET, missing, None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    server.stop().await;
}
