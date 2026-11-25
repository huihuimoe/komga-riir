//! Mihon read progress of a series (`SeriesController.getMihonReadProgressBySeriesId`/
//! `updateMihonReadProgressBySeriesId`).

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::books::library_with_books;
use crate::books::read_progress::{mark, progress, row, series_progress};
use crate::books::reading::access::create_reader;
use crate::books::reading::{fetch, pages_book};
use crate::support::db::content::book;
use crate::support::{ADMIN_EMAIL, TestServer, openapi};

const READER: &str = "reader@example.com";

fn route(series_id: &str) -> String {
    format!("/api/v2/series/{series_id}/read-progress/tachiyomi")
}

async fn get(server: &TestServer, series_id: &str) -> Value {
    let response = server
        .admin(Method::GET, &route(series_id))
        .send()
        .await
        .expect("Mihon progress request");
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::GET, response).await
}

async fn put(server: &TestServer, series_id: &str, body: &Value) -> StatusCode {
    let response = server
        .admin(Method::PUT, &route(series_id))
        .json(body)
        .send()
        .await
        .expect("Mihon progress update");
    let status = response.status();
    if status == StatusCode::NO_CONTENT {
        openapi::checked(Method::PUT, response).await;
    }
    status
}

fn counts(read: i32, unread: i32, in_progress: i32, last: f32) -> Value {
    json!({
        "booksCount": read + unread + in_progress,
        "booksReadCount": read,
        "booksUnreadCount": unread,
        "booksInProgressCount": in_progress,
        "lastReadContinuousNumberSort": last,
        "maxNumberSort": 4.0,
    })
}

#[tokio::test]
async fn books_up_to_a_number_are_read_for_one_user() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/v01.cbz", pages_book()),
            ("Series/v02.cbz", pages_book()),
            ("Series/v03.cbz", pages_book()),
            ("Series/v04.cbz", pages_book()),
        ],
    )
    .await;
    let (v01, series) = book(&pool, "v01").await;
    let v02 = book(&pool, "v02").await.0;
    let v03 = book(&pool, "v03").await.0;
    let reader = create_reader(&server, READER, json!({"roles": []})).await;
    mark(&server, &v01, json!({"completed": true})).await;
    mark(&server, &v02, json!({"page": 2})).await;
    let reader_mark = reader
        .request(
            &server,
            Method::PATCH,
            &format!("/api/v1/books/{v03}/read-progress"),
        )
        .json(&json!({"completed": true}));
    assert_eq!(fetch(reader_mark).await.status, StatusCode::NO_CONTENT);

    assert_eq!(get(&server, &series).await, counts(1, 2, 1, 1.0));

    for _ in 0..2 {
        assert_eq!(
            put(&server, &series, &json!({"lastBookNumberSortRead": 2.5})).await,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(progress(&pool, &v02).await, [row(ADMIN_EMAIL, 3, true)]);
    assert_eq!(progress(&pool, &v03).await, [row(READER, 3, true)]);
    assert_eq!(
        series_progress(&pool, &series).await,
        [(ADMIN_EMAIL.to_owned(), 2, 0), (READER.to_owned(), 1, 0)]
    );
    assert_eq!(get(&server, &series).await, counts(2, 2, 0, 2.0));

    // The reader's leading book is unread, so nothing counts as continuously read.
    let reader_view = fetch(reader.request(&server, Method::GET, &route(&series))).await;
    assert_eq!(reader_view.status, StatusCode::OK);
    let reader_view: Value = serde_json::from_slice(&reader_view.body).expect("JSON progress");
    assert_eq!(reader_view, counts(1, 3, 0, 0.0));
    server.stop().await;
}

#[tokio::test]
async fn unknown_hidden_and_invalid_requests_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/v01.cbz", pages_book())]).await;
    let (v01, series) = book(&pool, "v01").await;
    let update = json!({"lastBookNumberSortRead": 1});

    let unknown = fetch(server.admin(Method::GET, &route("0"))).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    // A user who sees every library is not checked against the series when updating.
    assert_eq!(put(&server, "0", &update).await, StatusCode::NO_CONTENT);
    assert_eq!(
        put(&server, &series, &json!({})).await,
        StatusCode::BAD_REQUEST
    );

    let outsider = create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    for method in [Method::GET, Method::PUT] {
        let request = |id: &str| {
            outsider
                .request(&server, method.clone(), &route(id))
                .json(&update)
        };
        assert_eq!(fetch(request("0")).await.status, StatusCode::NOT_FOUND);
        let denied = fetch(request(&series)).await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN, "{method}");
        assert_eq!(denied.message(), "403 FORBIDDEN");
        let anonymous = fetch(
            server
                .request(method.clone(), &route(&series))
                .json(&update),
        )
        .await;
        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED, "{method}");
    }
    assert!(progress(&pool, &v01).await.is_empty());
    server.stop().await;
}
