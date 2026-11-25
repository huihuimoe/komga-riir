//! `PUT /api/v1/books/thumbnails`: regenerating generated thumbnails after a size change.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use image::{ImageFormat, Rgb, RgbImage};
use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use super::library_with_books;
use super::reading::access::create_reader;
use super::thumbnails::book_with_first_page;
use crate::libraries::route;
use crate::support::TestServer;
use crate::support::db::content::book;
use crate::support::tasks::{tasks, wait_for_scan_work, wait_for_tasks};

const ROUTE: &str = "/api/v1/books/thumbnails";

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    RgbImage::from_pixel(width, height, Rgb([200, 0, 0]))
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("Encode cover");
    bytes
}

/// `(ID, WIDTH, HEIGHT)` of the book's generated thumbnail.
async fn generated(pool: &SqlitePool, book_id: &str) -> (String, i64, i64) {
    sqlx::query_as(
        "SELECT ID, WIDTH, HEIGHT FROM THUMBNAIL_BOOK WHERE BOOK_ID = ? AND TYPE = 'GENERATED'",
    )
    .bind(book_id)
    .fetch_one(pool)
    .await
    .expect("Generated thumbnail")
}

async fn set_size(server: &TestServer, size: &str) {
    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({"thumbnailSize": size}))
        .send()
        .await
        .expect("Settings update");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

/// Requests the regeneration and waits for the search and the generations it queued.
async fn regenerate(server: &TestServer, directory: &Path, query: &str) {
    let response = server
        .admin(Method::PUT, &format!("{ROUTE}{query}"))
        .send()
        .await
        .expect("Regeneration request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    wait_for_tasks(directory, |rows| {
        rows.iter().all(|row| {
            !row.2.ends_with("$FindBookThumbnailsToRegenerate")
                && !row.2.ends_with("$GenerateBookThumbnail")
        })
    })
    .await;
}

#[tokio::test]
async fn regeneration_follows_the_configured_size() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = library_with_books(
        directory.path(),
        &[
            ("Big/Big.cbz", book_with_first_page(&png(1200, 600))),
            ("Small/Small.cbz", book_with_first_page(&png(200, 100))),
        ],
    )
    .await;
    let (big, _) = book(&pool, "Big").await;
    let (small, _) = book(&pool, "Small").await;
    let first = generated(&pool, &big).await;
    assert_eq!((first.1, first.2), (300, 150));

    // Changing the size alone regenerates nothing.
    set_size(&server, "LARGE").await;
    assert_eq!(generated(&pool, &big).await, first);

    regenerate(&server, directory.path(), "?for_bigger_result_only=true").await;
    let large = generated(&pool, &big).await;
    assert_eq!((large.1, large.2), (900, 450));

    // Only thumbnails smaller than the new size on both edges are regenerated.
    set_size(&server, "MEDIUM").await;
    let small_before = generated(&pool, &small).await;
    regenerate(&server, directory.path(), "?for_bigger_result_only=true").await;
    assert_eq!(generated(&pool, &big).await, large, "900 px wide is kept");
    let small_after = generated(&pool, &small).await;
    assert_ne!(small_after.0, small_before.0, "regenerated");
    assert_eq!((small_after.1, small_after.2), (200, 100), "never enlarged");

    // Without the flag every book that is not soft-deleted is regenerated.
    fs::remove_file(directory.path().join("comics/Small/Small.cbz")).expect("Remove a book");
    let response = server
        .admin(Method::POST, &format!("{}/scan", route(&library)))
        .send()
        .await
        .expect("Scan request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    wait_for_scan_work(directory.path()).await;
    regenerate(&server, directory.path(), "").await;
    let medium = generated(&pool, &big).await;
    assert_eq!((medium.1, medium.2), (600, 300));
    assert_eq!(generated(&pool, &small).await, small_after, "deleted book");
    server.stop().await;
}

#[tokio::test]
async fn regeneration_is_for_admins_only() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    create_reader(&server, "reader@example.com", json!({"roles": []})).await;
    let as_reader = |query: &str| {
        server
            .request(Method::PUT, &format!("{ROUTE}{query}"))
            .basic_auth("reader@example.com", Some("secret"))
    };
    for (request, status) in [
        (server.request(Method::PUT, ROUTE), StatusCode::UNAUTHORIZED),
        (as_reader(""), StatusCode::FORBIDDEN),
        // Arguments bind before `@PreAuthorize` runs.
        (
            as_reader("?for_bigger_result_only=maybe"),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(request.send().await.expect("Request").status(), status);
    }
    assert!(
        tasks(directory.path()).await.is_empty(),
        "rejected requests queue nothing"
    );
    server.stop().await;
}
