//! `GET /api/v1/books/{id}` and its neighbours in the series (`previous`, `next`).

use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::reading::access::create_reader;
use super::reading::fetch;
use super::{PNG, library_with_books};
use crate::read_lists::rate;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

/// The book as the admin gets it from `path`, or the failure status.
async fn get(server: &TestServer, path: &str) -> Result<Value, StatusCode> {
    let response = server
        .admin(Method::GET, path)
        .send()
        .await
        .expect("Book request");
    match response.status() {
        StatusCode::OK => Ok(openapi::checked(Method::GET, response).await),
        status => Err(status),
    }
}

fn id(book: &Result<Value, StatusCode>) -> &str {
    book.as_ref().expect("Book")["id"]
        .as_str()
        .expect("Book ID")
}

async fn set_number_sort(server: &TestServer, book_id: &str, number_sort: f64) {
    let response = server
        .admin(Method::PATCH, &format!("/api/v1/books/{book_id}/metadata"))
        .json(&json!({"numberSort": number_sort}))
        .send()
        .await
        .expect("Metadata edit");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn books_are_shown_to_users_who_may_see_them() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let file = cbz::zip(&[("page.png", STORED, &cbz::fixture(PNG))]);
    let (server, library, pool) =
        library_with_books(directory.path(), &[("Series/one.cbz", file)]).await;
    let (book_id, series_id) = book(&pool, "one").await;
    let path = format!("/api/v1/books/{book_id}");

    let details = get(&server, &path).await.expect("Book");
    assert_eq!(details["id"], book_id.as_str());
    assert_eq!(details["seriesId"], series_id.as_str());
    assert_eq!(details["seriesTitle"], "Series");
    assert_eq!(details["libraryId"], library["id"]);
    assert_eq!(details["media"]["pagesCount"], 1);
    let url = Path::new(details["url"].as_str().expect("Book URL"));
    assert!(
        url.is_absolute() && url.ends_with("Series/one.cbz"),
        "{}",
        url.display()
    );
    assert_eq!(
        get(&server, "/api/v1/books/missing").await,
        Err(StatusCode::NOT_FOUND)
    );

    let reader = create_reader(
        &server,
        "reader@example.com",
        json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}}),
    )
    .await;
    let seen = fetch(reader.get(&server, &path)).await;
    assert_eq!(seen.status, StatusCode::OK);
    let seen: Value = serde_json::from_slice(&seen.body).expect("JSON book");
    assert_eq!(
        seen["url"], "one.cbz",
        "only admins see the path of the file"
    );
    rate(&server, &series_id, 18).await;
    assert_eq!(
        fetch(reader.get(&server, &path)).await.status,
        StatusCode::FORBIDDEN
    );
    let missing = fetch(reader.get(&server, "/api/v1/books/missing")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let elsewhere = create_reader(
        &server,
        "elsewhere@example.com",
        json!({"sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    assert_eq!(
        fetch(elsewhere.get(&server, &path)).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fetch(server.request(Method::GET, &path)).await.status,
        StatusCode::UNAUTHORIZED
    );
    server.stop().await;
}

#[tokio::test]
async fn neighbours_follow_the_number_sort_of_the_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let file = cbz::zip(&[("page.png", STORED, &cbz::fixture(PNG))]);
    let books = [
        "S/b1.cbz",
        "S/b2.cbz",
        "S/b3.cbz",
        "S/b4.cbz",
        "Other/o1.cbz",
    ]
    .map(|path| (path, file.clone()));
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let (b1, series_id) = book(&pool, "b1").await;
    let (b2, _) = book(&pool, "b2").await;
    let (b3, _) = book(&pool, "b3").await;
    let (b4, _) = book(&pool, "b4").await;
    let sibling = |id: &str, which: &str| format!("/api/v1/books/{id}/{which}");

    assert_eq!(id(&get(&server, &sibling(&b1, "next")).await), b2);
    assert_eq!(id(&get(&server, &sibling(&b3, "previous")).await), b2);
    assert_eq!(
        get(&server, &sibling(&b1, "previous")).await,
        Err(StatusCode::NOT_FOUND)
    );
    assert_eq!(
        get(&server, &sibling(&b4, "next")).await,
        Err(StatusCode::NOT_FOUND)
    );
    assert_eq!(
        get(&server, &sibling("missing", "next")).await,
        Err(StatusCode::NOT_FOUND)
    );

    // b4 moves before b1; b2 and b3 share a number and follow each other by ID.
    set_number_sort(&server, &b4, 0.5).await;
    set_number_sort(&server, &b3, 2.0).await;
    assert_eq!(id(&get(&server, &sibling(&b1, "previous")).await), b4);
    let (low, high) = if b2 < b3 { (&b2, &b3) } else { (&b3, &b2) };
    assert_eq!(id(&get(&server, &sibling(&b1, "next")).await), *low);
    assert_eq!(id(&get(&server, &sibling(low, "next")).await), *high);
    assert_eq!(id(&get(&server, &sibling(high, "previous")).await), *low);
    assert_eq!(
        get(&server, &sibling(high, "next")).await,
        Err(StatusCode::NOT_FOUND)
    );

    let reader = create_reader(
        &server,
        "reader@example.com",
        json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}}),
    )
    .await;
    let next = fetch(reader.get(&server, &sibling(&b4, "next"))).await;
    assert_eq!(next.status, StatusCode::OK);
    rate(&server, &series_id, 18).await;
    let next = fetch(reader.get(&server, &sibling(&b4, "next"))).await;
    assert_eq!(next.status, StatusCode::FORBIDDEN);
    let missing = fetch(reader.get(&server, &sibling("missing", "next"))).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    server.stop().await;
}
