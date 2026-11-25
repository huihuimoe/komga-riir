//! Read progress of one book (`BookController.markBookReadProgress`/`deleteBookReadProgress`).

use flate2::read::GzDecoder;
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::epub::{divina_book, sample};
use super::reading::access::create_reader;
use super::reading::{fetch, pages_book};
use super::{PNG, library_with_books};
use crate::support::cbz;
use crate::support::db::content::book;
use crate::support::{ADMIN_EMAIL, TestServer, openapi};

const READER: &str = "reader@example.com";

/// `(EMAIL, PAGE, COMPLETED, DEVICE_ID, DEVICE_NAME)` of each progress on a book, by email.
type Progress = (String, i32, bool, String, String);

pub(crate) async fn progress(pool: &SqlitePool, book_id: &str) -> Vec<Progress> {
    sqlx::query_as(
        "SELECT u.EMAIL, r.PAGE, r.COMPLETED, r.DEVICE_ID, r.DEVICE_NAME FROM READ_PROGRESS r \
         JOIN USER u ON u.ID = r.USER_ID WHERE r.BOOK_ID = ? ORDER BY u.EMAIL",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .expect("Read progress")
}

/// `(EMAIL, READ_COUNT, IN_PROGRESS_COUNT)` of each series progress, by email.
pub(crate) async fn series_progress(pool: &SqlitePool, series_id: &str) -> Vec<(String, i32, i32)> {
    sqlx::query_as(
        "SELECT u.EMAIL, r.READ_COUNT, r.IN_PROGRESS_COUNT FROM READ_PROGRESS_SERIES r \
         JOIN USER u ON u.ID = r.USER_ID WHERE r.SERIES_ID = ? ORDER BY u.EMAIL",
    )
    .bind(series_id)
    .fetch_all(pool)
    .await
    .expect("Series read progress")
}

/// The stored locator of the user's progress on the book, decompressed.
async fn locator(pool: &SqlitePool, book_id: &str, email: &str) -> Option<Value> {
    let blob: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT r.LOCATOR FROM READ_PROGRESS r JOIN USER u ON u.ID = r.USER_ID \
         WHERE r.BOOK_ID = ? AND u.EMAIL = ?",
    )
    .bind(book_id)
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("Stored locator");
    blob.map(|blob| serde_json::from_reader(GzDecoder::new(blob.as_slice())).expect("JSON locator"))
}

fn route(book_id: &str) -> String {
    format!("/api/v1/books/{book_id}/read-progress")
}

/// Marks the admin's progress on the book with `body`.
pub(crate) async fn mark(server: &TestServer, book_id: &str, body: Value) -> Response {
    send(server.admin(Method::PATCH, &route(book_id)).json(&body)).await
}

async fn send(request: RequestBuilder) -> Response {
    request.send().await.expect("Read progress request")
}

pub(crate) fn row(email: &str, page: i32, completed: bool) -> Progress {
    (
        email.to_owned(),
        page,
        completed,
        String::new(),
        String::new(),
    )
}

#[tokio::test]
async fn pages_and_completion_are_kept_per_user_with_their_series_progress() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/one.cbz", pages_book()),
            ("Series/two.cbz", pages_book()),
        ],
    )
    .await;
    let (one, series) = book(&pool, "one").await;
    let two = book(&pool, "two").await.0;
    let reader = create_reader(&server, READER, json!({"roles": []})).await;

    let response = mark(&server, &one, json!({"page": 2})).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    openapi::checked(Method::PATCH, response).await;
    assert_eq!(progress(&pool, &one).await, [row(ADMIN_EMAIL, 2, false)]);
    assert_eq!(locator(&pool, &one, ADMIN_EMAIL).await, None);
    assert_eq!(
        series_progress(&pool, &series).await,
        [(ADMIN_EMAIL.to_owned(), 0, 1)]
    );

    // The last page completes the book; `completed: true` goes to the last page whatever the page.
    mark(&server, &one, json!({"page": 3, "completed": false})).await;
    assert_eq!(progress(&pool, &one).await, [row(ADMIN_EMAIL, 3, true)]);
    for _ in 0..2 {
        let response = mark(&server, &two, json!({"page": 1, "completed": true})).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
    assert_eq!(progress(&pool, &two).await, [row(ADMIN_EMAIL, 3, true)]);
    assert_eq!(
        series_progress(&pool, &series).await,
        [(ADMIN_EMAIL.to_owned(), 2, 0)]
    );

    // Another user's progress is their own.
    let response = send(
        reader
            .request(&server, Method::PATCH, &route(&one))
            .json(&json!({"page": 1})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        progress(&pool, &one).await,
        [row(ADMIN_EMAIL, 3, true), row(READER, 1, false)]
    );
    let response = send(reader.request(&server, Method::DELETE, &route(&two))).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(progress(&pool, &two).await, [row(ADMIN_EMAIL, 3, true)]);

    for _ in 0..2 {
        let response = send(server.admin(Method::DELETE, &route(&one))).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        openapi::checked(Method::DELETE, response).await;
    }
    assert_eq!(progress(&pool, &one).await, [row(READER, 1, false)]);
    assert_eq!(
        series_progress(&pool, &series).await,
        [(ADMIN_EMAIL.to_owned(), 1, 0), (READER.to_owned(), 0, 1)]
    );
    server.stop().await;
}

#[tokio::test]
async fn invalid_updates_unknown_and_hidden_books_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/one.cbz", pages_book())]).await;
    let one = book(&pool, "one").await.0;

    // Bodies are checked before the book is looked up.
    for (body, violations) in [
        (
            json!({"page": 0}),
            json!([{"fieldName": "page", "message": "must be greater than 0"}]),
        ),
        (
            json!({"page": -1, "completed": true}),
            json!([{"fieldName": "page", "message": "must be greater than 0"}]),
        ),
        (json!({}), json!([])),
        (json!({"completed": false}), json!([])),
    ] {
        let response = mark(&server, "0", body.clone()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            openapi::checked(Method::PATCH, response).await,
            json!({"violations": violations}),
            "{body}"
        );
    }
    let malformed = server
        .admin(Method::PATCH, &route(&one))
        .header("Content-Type", "application/json")
        .body("{\"page\":");
    assert_eq!(send(malformed).await.status(), StatusCode::BAD_REQUEST);

    let beyond = fetch(
        server
            .admin(Method::PATCH, &route(&one))
            .json(&json!({"page": 4})),
    )
    .await;
    assert_eq!(beyond.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        beyond.message(),
        "Page argument (4) must be within 1 and book page count (3)"
    );
    assert_eq!(
        mark(&server, "0", json!({"page": 1})).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(server.admin(Method::DELETE, &route("0")))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    let outsider = create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    for method in [Method::PATCH, Method::DELETE] {
        let denied = fetch(
            outsider
                .request(&server, method.clone(), &route(&one))
                .json(&json!({"completed": true})),
        )
        .await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN, "{method}");
        assert_eq!(denied.message(), "403 FORBIDDEN");
    }
    let anonymous = server.request(Method::DELETE, &route(&one));
    assert_eq!(send(anonymous).await.status(), StatusCode::UNAUTHORIZED);
    assert!(progress(&pool, &one).await.is_empty());
    server.stop().await;
}

#[tokio::test]
async fn divina_epub_pages_keep_their_position_and_other_epubs_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/divina.epub", divina_book(&cbz::fixture(PNG))),
            (
                "Series/theft.epub",
                sample("The Incomplete Theft - Ralph Burke.epub"),
            ),
        ],
    )
    .await;
    let divina = book(&pool, "divina").await.0;
    let positions = fetch(server.admin(Method::GET, &format!("/api/v1/books/{divina}/positions")));
    let positions: Value = serde_json::from_slice(&positions.await.body).expect("JSON positions");

    mark(&server, &divina, json!({"page": 2})).await;
    assert_eq!(progress(&pool, &divina).await, [row(ADMIN_EMAIL, 2, true)]);
    assert_eq!(
        locator(&pool, &divina, ADMIN_EMAIL).await.as_ref(),
        Some(&positions["positions"][1])
    );
    // Completing a book keeps no locator.
    mark(&server, &divina, json!({"completed": true})).await;
    assert_eq!(locator(&pool, &divina, ADMIN_EMAIL).await, None);

    let theft = book(&pool, "theft").await.0;
    let rejected = fetch(
        server
            .admin(Method::PATCH, &route(&theft))
            .json(&json!({"page": 1})),
    )
    .await;
    assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
    assert_eq!(rejected.message(), "epub book is not Divina compatible");
    // Completion needs no position.
    assert_eq!(
        mark(&server, &theft, json!({"completed": true}))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    server.stop().await;
}
