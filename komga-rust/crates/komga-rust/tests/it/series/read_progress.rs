//! Read progress of a whole series (`SeriesController.markSeriesAsRead`/`markSeriesAsUnread`).

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use crate::books::library_with_books;
use crate::books::read_progress::{mark, progress, row, series_progress};
use crate::books::reading::access::create_reader;
use crate::books::reading::{fetch, pages_book};
use crate::support::db::content::book;
use crate::support::{ADMIN_EMAIL, TestServer, openapi};

const READER: &str = "reader@example.com";

fn route(series_id: &str) -> String {
    format!("/api/v1/series/{series_id}/read-progress")
}

async fn read_date(pool: &SqlitePool, book_id: &str, email: &str) -> String {
    sqlx::query_scalar(
        "SELECT r.READ_DATE FROM READ_PROGRESS r JOIN USER u ON u.ID = r.USER_ID \
         WHERE r.BOOK_ID = ? AND u.EMAIL = ?",
    )
    .bind(book_id)
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("Read date")
}

async fn send(server: &TestServer, method: Method, series_id: &str) -> StatusCode {
    let response = server
        .admin(method.clone(), &route(series_id))
        .send()
        .await
        .expect("Series read progress request");
    let status = response.status();
    if status == StatusCode::NO_CONTENT {
        openapi::checked(method, response).await;
    }
    status
}

#[tokio::test]
async fn a_series_is_read_and_unread_for_one_user() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/one.cbz", pages_book()),
            ("Series/two.cbz", pages_book()),
            ("Series/three.cbz", pages_book()),
        ],
    )
    .await;
    let (one, series) = book(&pool, "one").await;
    let two = book(&pool, "two").await.0;
    let three = book(&pool, "three").await.0;
    let reader = create_reader(&server, READER, json!({"roles": []})).await;
    mark(&server, &one, json!({"page": 2})).await;
    mark(&server, &two, json!({"completed": true})).await;
    let completed_at = read_date(&pool, &two, ADMIN_EMAIL).await;
    let reader_mark = reader
        .request(
            &server,
            Method::PATCH,
            &format!("/api/v1/books/{three}/read-progress"),
        )
        .json(&json!({"page": 1}));
    assert_eq!(fetch(reader_mark).await.status, StatusCode::NO_CONTENT);
    tokio::time::sleep(Duration::from_millis(5)).await;

    // Unfinished books go to their last page; a completed book keeps its read date.
    for _ in 0..2 {
        assert_eq!(
            send(&server, Method::POST, &series).await,
            StatusCode::NO_CONTENT
        );
    }
    for id in [&one, &two] {
        assert_eq!(progress(&pool, id).await, [row(ADMIN_EMAIL, 3, true)]);
    }
    assert_eq!(
        progress(&pool, &three).await,
        [row(ADMIN_EMAIL, 3, true), row(READER, 1, false)]
    );
    assert_eq!(read_date(&pool, &two, ADMIN_EMAIL).await, completed_at);
    assert_ne!(read_date(&pool, &one, ADMIN_EMAIL).await, completed_at);
    assert_eq!(
        series_progress(&pool, &series).await,
        [(ADMIN_EMAIL.to_owned(), 3, 0), (READER.to_owned(), 0, 1)]
    );

    for _ in 0..2 {
        assert_eq!(
            send(&server, Method::DELETE, &series).await,
            StatusCode::NO_CONTENT
        );
    }
    for id in [&one, &two] {
        assert!(progress(&pool, id).await.is_empty());
    }
    assert_eq!(progress(&pool, &three).await, [row(READER, 1, false)]);
    assert_eq!(
        series_progress(&pool, &series).await,
        [(READER.to_owned(), 0, 1)]
    );
    server.stop().await;
}

#[tokio::test]
async fn unknown_and_hidden_series_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/one.cbz", pages_book())]).await;
    let (one, series) = book(&pool, "one").await;

    // A user who sees every library is not checked against the series.
    assert_eq!(
        send(&server, Method::POST, "0").await,
        StatusCode::NO_CONTENT
    );
    let outsider = create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    for method in [Method::POST, Method::DELETE] {
        let unknown = fetch(outsider.request(&server, method.clone(), &route("0"))).await;
        assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{method}");
        let denied = fetch(outsider.request(&server, method.clone(), &route(&series))).await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN, "{method}");
        assert_eq!(denied.message(), "403 FORBIDDEN");
        let anonymous = fetch(server.request(method.clone(), &route(&series))).await;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED, "{method}");
    }
    assert!(progress(&pool, &one).await.is_empty());
    server.stop().await;
}
