//! `POST …/metadata/refresh` of a book, a series, and a library.

use std::fs;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::books::fields;
use super::{PNG, assert_no_failed_task, comic, library, write};
use crate::libraries::{admin_json, route};
use crate::support::cbz;
use crate::support::db::content::book;
use crate::support::tasks::wait_for_scan_work;
use crate::support::{TestServer, json};

fn titled(title: &str) -> Vec<u8> {
    comic(Some(&format!(
        "<ComicInfo><Title>{title}</Title></ComicInfo>"
    )))
}

async fn request(server: &TestServer, directory: &Path, path: &str) {
    let response = server
        .admin(Method::POST, path)
        .send()
        .await
        .expect("Refresh request");
    assert_eq!(response.status(), StatusCode::ACCEPTED, "{path}");
    assert_no_failed_task(wait_for_scan_work(directory).await);
}

/// How many sidecar thumbnails `GET {owner}/thumbnails` lists.
async fn sidecars(server: &TestServer, owner: &str) -> usize {
    let response = server
        .admin(Method::GET, &format!("{owner}/thumbnails"))
        .send()
        .await
        .expect("Thumbnails request");
    assert_eq!(response.status(), StatusCode::OK);
    let thumbnails: Value = json(response).await;
    thumbnails
        .as_array()
        .expect("Thumbnail list")
        .iter()
        .filter(|thumbnail| thumbnail["type"] == "SIDECAR")
        .count()
}

async fn title(pool: &SqlitePool, book_id: &str) -> String {
    fields(pool, book_id).await.0
}

#[tokio::test]
async fn each_refresh_request_rereads_metadata_and_local_artwork() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = directory.path();
    write(root, "Series/book.cbz", &titled("First"));
    let (server, library, pool) = library(root).await;
    let (book_id, series_id) = book(&pool, "book").await;
    let book_route = format!("/api/v1/books/{book_id}");
    let series_route = format!("/api/v1/series/{series_id}");

    // No scan sees these changes: only the refresh requests read them.
    write(root, "Series/book.cbz", &titled("Book"));
    write(root, "Series/book.png", &cbz::fixture(PNG));
    request(&server, root, &format!("{book_route}/metadata/refresh")).await;
    assert_eq!(title(&pool, &book_id).await, "Book");
    assert_eq!(sidecars(&server, &book_route).await, 1);

    write(root, "Series/book.cbz", &titled("Series"));
    write(root, "Series/cover.png", &cbz::fixture(PNG));
    request(&server, root, &format!("{series_route}/metadata/refresh")).await;
    assert_eq!(title(&pool, &book_id).await, "Series");
    assert_eq!(sidecars(&server, &series_route).await, 1);

    write(root, "Series/book.cbz", &titled("Library"));
    fs::remove_file(root.join("comics/Series/book.png")).expect("Remove book artwork");
    fs::remove_file(root.join("comics/Series/cover.png")).expect("Remove series artwork");
    request(
        &server,
        root,
        &format!("{}/metadata/refresh", route(&library)),
    )
    .await;
    assert_eq!(title(&pool, &book_id).await, "Library");
    assert_eq!(sidecars(&server, &book_route).await, 0);
    assert_eq!(sidecars(&server, &series_route).await, 0);
    server.stop().await;
}

#[tokio::test]
async fn refresh_requests_need_an_admin_and_an_existing_book_only() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    write(directory.path(), "Series/book.cbz", &comic(None));
    let (server, library, pool) = library(directory.path()).await;
    let (book_id, series_id) = book(&pool, "book").await;
    let reader = json!({"email": "reader@example.com", "password": "readerpass"});
    admin_json(
        &server,
        Method::POST,
        "/api/v2/users",
        Some(&reader),
        StatusCode::CREATED,
    )
    .await;

    for path in [
        format!("/api/v1/books/{book_id}/metadata/refresh"),
        format!("/api/v1/series/{series_id}/metadata/refresh"),
        format!("{}/metadata/refresh", route(&library)),
    ] {
        let response = server
            .request(Method::POST, &path)
            .basic_auth("reader@example.com", Some("readerpass"))
            .send()
            .await
            .expect("Reader request");
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    // Kotlin looks up only the book; an unknown series or library has nothing to refresh.
    for (path, status) in [
        (
            "/api/v1/books/UNKNOWN/metadata/refresh",
            StatusCode::NOT_FOUND,
        ),
        (
            "/api/v1/series/UNKNOWN/metadata/refresh",
            StatusCode::ACCEPTED,
        ),
        (
            "/api/v1/libraries/UNKNOWN/metadata/refresh",
            StatusCode::ACCEPTED,
        ),
    ] {
        let response = server
            .admin(Method::POST, path)
            .send()
            .await
            .expect("Admin request");
        assert_eq!(response.status(), status, "{path}");
    }
    server.stop().await;
}
