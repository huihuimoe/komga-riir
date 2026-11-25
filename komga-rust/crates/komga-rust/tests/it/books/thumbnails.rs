//! Generated book thumbnails: made after analysis, served from the database, and regenerated
//! when lost.

use std::fs;
use std::io::Cursor;

use image::{GenericImageView, ImageFormat, Rgb, RgbImage};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use super::reading::access::create_reader;
use super::reading::{Fetched, fetch};
use super::{analyze, library_with_books};
use crate::support::TestServer;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::tasks::{GENERATE_BOOK_THUMBNAIL, wait_for_generations, wait_for_tasks};

/// An 800 x 400 PNG cover.
pub(crate) fn cover() -> Vec<u8> {
    let image = RgbImage::from_fn(800, 400, |x, _| {
        Rgb([u8::try_from(x / 4).unwrap_or(0), 0, 0])
    });
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("Encode cover");
    bytes
}

pub(crate) fn book_with_first_page(first: &[u8]) -> Vec<u8> {
    cbz::zip(&[("01.png", STORED, first), ("02.png", STORED, &cover())])
}

/// `(ID, TYPE, SELECTED, MEDIA_TYPE, WIDTH, HEIGHT, FILE_SIZE, LENGTH(THUMBNAIL))` of a book.
type ThumbnailRow = (String, String, bool, String, i64, i64, i64, i64);

async fn thumbnails(pool: &SqlitePool, book_id: &str) -> Vec<ThumbnailRow> {
    sqlx::query_as(
        "SELECT ID, TYPE, SELECTED, MEDIA_TYPE, WIDTH, HEIGHT, FILE_SIZE, LENGTH(THUMBNAIL) \
         FROM THUMBNAIL_BOOK WHERE BOOK_ID = ?",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .expect("Thumbnail rows")
}

async fn get_thumbnail(server: &TestServer, book_id: &str) -> Fetched {
    fetch(server.admin(Method::GET, &format!("/api/v1/books/{book_id}/thumbnail"))).await
}

pub(crate) fn assert_jpeg(thumbnail: &Fetched, size: (u32, u32)) {
    assert_eq!(thumbnail.status, StatusCode::OK);
    assert_eq!(thumbnail.header(&CONTENT_TYPE), "image/jpeg");
    let image = image::load_from_memory_with_format(&thumbnail.body, ImageFormat::Jpeg)
        .expect("Thumbnail decodes");
    assert_eq!(image.dimensions(), size);
}

#[tokio::test]
async fn generated_thumbnails_are_served_without_reading_the_book() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[("Série [x]/Vol é 1.cbz", book_with_first_page(&cover()))],
    )
    .await;
    wait_for_generations(directory.path()).await;
    let (book_id, _) = book(&pool, "Vol é 1").await;

    let thumbnail = get_thumbnail(&server, &book_id).await;
    assert_jpeg(&thumbnail, (300, 150));
    let rows = thumbnails(&pool, &book_id).await;
    let size = i64::try_from(thumbnail.body.len()).expect("Thumbnail size");
    assert_eq!(
        rows.iter()
            .map(|row| (&*row.1, row.2, &*row.3, row.4, row.5, row.6, row.7))
            .collect::<Vec<_>>(),
        [("GENERATED", true, "image/jpeg", 300, 150, size, size)]
    );

    // A hit reads the database only: the moved book file is not needed.
    let path = directory.path().join("comics/Série [x]/Vol é 1.cbz");
    let moved = path.with_extension("moved");
    fs::rename(&path, &moved).expect("Move the book away");
    let hit = get_thumbnail(&server, &book_id).await;
    assert_eq!(hit.status, StatusCode::OK);
    assert!(hit.body == thumbnail.body, "the stored thumbnail");
    fs::rename(&moved, &path).expect("Move the book back");

    // A new analysis replaces the generated thumbnail, at the current size.
    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({"thumbnailSize": "MEDIUM"}))
        .send()
        .await
        .expect("Settings update");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    analyze(&server, &book_id).await;
    wait_for_tasks(directory.path(), |rows| {
        rows.iter()
            .all(|row| row.2 != GENERATE_BOOK_THUMBNAIL && !row.2.ends_with("$AnalyzeBook"))
    })
    .await;
    assert_jpeg(&get_thumbnail(&server, &book_id).await, (600, 300));
    let replaced = thumbnails(&pool, &book_id).await;
    assert_eq!(replaced.len(), 1, "one generated thumbnail per book");
    assert_ne!(replaced[0].0, rows[0].0, "a new thumbnail");
    assert!(replaced[0].2, "selected");
    server.stop().await;
}

#[tokio::test]
async fn missing_thumbnails_are_generated_once() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let mut truncated = cover();
    truncated.truncate(truncated.len() / 2);
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Good/Good.cbz", book_with_first_page(&cover())),
            ("Bad/Bad.cbz", book_with_first_page(&truncated)),
        ],
    )
    .await;
    let tasks = wait_for_generations(directory.path()).await;
    let (good, _) = book(&pool, "Good").await;
    let (bad, _) = book(&pool, "Bad").await;
    let failed = format!("GENERATE_BOOK_THUMBNAIL_{bad}");
    assert_eq!(
        tasks
            .iter()
            .filter(|row| row.2 == GENERATE_BOOK_THUMBNAIL)
            .map(|row| &*row.0)
            .collect::<Vec<_>>(),
        [failed.as_str()],
        "only the undecodable cover fails"
    );

    // A lost thumbnail is answered as missing and generated again.
    sqlx::query("DELETE FROM THUMBNAIL_BOOK WHERE BOOK_ID = ?")
        .bind(&good)
        .execute(&pool)
        .await
        .expect("Lose the thumbnail");
    assert_eq!(
        get_thumbnail(&server, &good).await.status,
        StatusCode::NOT_FOUND
    );
    wait_for_generations(directory.path()).await;
    assert_jpeg(&get_thumbnail(&server, &good).await, (300, 150));

    // A failed generation is not retried by requests.
    for _ in 0..3 {
        assert_eq!(
            get_thumbnail(&server, &bad).await.status,
            StatusCode::NOT_FOUND
        );
    }
    let after = wait_for_generations(directory.path()).await;
    assert_eq!(after, tasks, "the failed task is left as it was");
    assert!(thumbnails(&pool, &bad).await.is_empty());

    // Any user who sees the book gets its thumbnail; others do not.
    let route = format!("/api/v1/books/{good}/thumbnail");
    let reader = create_reader(&server, "reader@example.com", json!({"roles": []})).await;
    assert_eq!(
        fetch(reader.get(&server, &route)).await.status,
        StatusCode::OK
    );
    let outsider = create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    assert_eq!(
        fetch(outsider.get(&server, &route)).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get_thumbnail(&server, "0").await.status,
        StatusCode::NOT_FOUND
    );
    let anonymous = server
        .request(Method::GET, &route)
        .send()
        .await
        .expect("Request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    server.stop().await;
}
