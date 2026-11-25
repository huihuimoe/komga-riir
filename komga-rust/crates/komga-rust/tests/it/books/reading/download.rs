//! Book file downloads.

use std::fs;

use reqwest::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, RANGE,
};
use reqwest::{Method, StatusCode};
use serde_json::json;

use super::access::{create_reader, with_role};
use super::{CACHE_PRIVATE, fetch, pages_book};
use crate::books::{PNG, library_with_books};
use crate::support::cbz::{self, ZSTD};
use crate::support::db::content::book;
use crate::support::db::seed;

/// Spring's attachment `Content-Disposition` of an ASCII file name.
fn attachment(file: &str) -> String {
    format!("attachment; filename=\"=?UTF-8?Q?{file}?=\"; filename*=UTF-8''{file}")
}

#[tokio::test]
async fn book_files_are_streamed_whole_whatever_their_media() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let zstd = cbz::zip(&[("page.png", ZSTD, &cbz::fixture(PNG))]);
    let text = b"plain text, not a book".to_vec();
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/Série é.cbz", pages_book()),
            ("text.cbz", text.clone()),
            ("zstd.cbz", zstd.clone()),
        ],
    )
    .await;
    let (book_id, _) = book(&pool, "Série é").await;
    let file = format!("/api/v1/books/{book_id}/file");
    let content = pages_book();

    let opds_v1 = format!("/opds/v1.2/books/{book_id}/file");
    let opds_v2 = format!("/opds/v2/books/{book_id}/file");
    for route in [
        file.clone(),
        format!("{file}/"),
        format!("{file}/any%20name.cbz"),
        format!("{opds_v1}/"),
        format!("{opds_v1}/any%20name.cbz"),
        opds_v2.clone(),
        format!("{opds_v2}/"),
        format!("{opds_v2}/any%20name.cbz"),
    ] {
        let response = fetch(server.admin(Method::GET, &route).header(RANGE, "bytes=0-9")).await;
        assert_eq!(response.status, StatusCode::OK, "{route}");
        assert!(response.body == content, "{route}: file bytes");
        assert_eq!(response.header(&CONTENT_TYPE), "application/zip", "{route}");
        assert_eq!(
            response.header(&CONTENT_LENGTH),
            content.len().to_string(),
            "{route}"
        );
        assert_eq!(
            response.header(&CONTENT_DISPOSITION),
            "attachment; filename=\"=?UTF-8?Q?S=C3=A9rie_=C3=A9.cbz?=\"; \
             filename*=UTF-8''S%C3%A9rie%20%C3%A9.cbz",
            "{route}"
        );
        assert_eq!(response.header(&CONTENT_RANGE), "", "{route}");
        assert_eq!(
            response.header(&ETAG),
            "",
            "{route}: not buffered for an ETag"
        );
        assert_eq!(response.header(&CACHE_CONTROL), CACHE_PRIVATE, "{route}");
    }
    let head = fetch(server.admin(Method::HEAD, &file)).await;
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(head.header(&CONTENT_LENGTH), content.len().to_string());
    assert!(head.body.is_empty(), "HEAD has no body");
    let nested = fetch(server.admin(Method::GET, &format!("{file}/a/b"))).await;
    assert_eq!(nested.status, StatusCode::NOT_FOUND);

    // Unsupported and not yet analyzed books download too; an unknown media type is binary.
    for (name, bytes, media_type) in [
        ("text", text, "text/plain"),
        ("zstd", zstd, "application/octet-stream"),
    ] {
        let (id, _) = book(&pool, name).await;
        let response = fetch(server.admin(Method::GET, &format!("/api/v1/books/{id}/file"))).await;
        assert_eq!(response.status, StatusCode::OK, "{name}");
        assert!(response.body == bytes, "{name}: file bytes");
        assert_eq!(response.header(&CONTENT_TYPE), media_type, "{name}");
        assert_eq!(
            response.header(&CONTENT_DISPOSITION),
            attachment(&format!("{name}.cbz")),
            "{name}"
        );
    }
    server.stop().await;
}

#[tokio::test]
async fn book_files_need_file_download_and_visible_content() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/book.cbz", pages_book()),
            ("moved.cbz", pages_book()),
        ],
    )
    .await;
    let (book_id, series_id) = book(&pool, "book").await;
    seed::series_age_rating(&pool, &series_id, 16).await;
    let file = format!("/api/v1/books/{book_id}/file");
    let missing = "/api/v1/books/missing/file";

    let unauthenticated = fetch(server.request(Method::GET, &file)).await;
    assert_eq!(unauthenticated.status, StatusCode::UNAUTHORIZED);

    // The role is checked before the book lookup.
    let no_role = create_reader(&server, "no-role@example.com", json!({"roles": []})).await;
    for path in [file.as_str(), missing] {
        let response = fetch(no_role.get(&server, path)).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(response.message(), "Forbidden", "{path}");
    }
    let downloading = |extra| with_role("FILE_DOWNLOAD", extra);
    let denied = [
        downloading(json!({"sharedLibraries": {"all": false, "libraryIds": []}})),
        downloading(json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}})),
        downloading(json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}})),
    ];
    for (index, access) in denied.into_iter().enumerate() {
        let reader = create_reader(&server, &format!("denied-{index}@example.com"), access).await;
        let response = fetch(reader.get(&server, &file)).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{index}");
        assert_eq!(response.message(), "403 FORBIDDEN", "{index}");
    }
    let allowed = downloading(json!({"ageRestriction": {"age": 16, "restriction": "ALLOW_ONLY"}}));
    let reader = create_reader(&server, "allowed@example.com", allowed).await;
    let response = fetch(reader.get(&server, &file)).await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body == pages_book(), "file bytes");

    let not_found = fetch(server.admin(Method::GET, missing)).await;
    assert_eq!(not_found.status, StatusCode::NOT_FOUND);
    assert_eq!(not_found.message(), "404 NOT_FOUND");
    let (moved, _) = book(&pool, "moved").await;
    fs::remove_file(directory.path().join("comics/moved.cbz")).expect("Move book away");
    let gone = fetch(server.admin(Method::GET, &format!("/api/v1/books/{moved}/file"))).await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert_eq!(gone.message(), "File not found, it may have moved");
    server.stop().await;
}
