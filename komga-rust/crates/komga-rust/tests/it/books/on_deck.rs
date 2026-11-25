//! Books on deck (`BookController.getBooksOnDeck`).

use std::path::Path;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::library_with_books;
use super::read_progress::mark;
use super::reading::access::{Reader, create_reader};
use super::reading::{fetch, pages_book};
use crate::read_lists::rate;
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

const ON_DECK: &str = "/api/v1/books/ondeck";
const READER: &str = "reader@example.com";

async fn on_deck(server: &TestServer, query: &str) -> Value {
    let response = server
        .admin(Method::GET, &format!("{ON_DECK}{query}"))
        .send()
        .await
        .expect("On deck request");
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::GET, response).await
}

fn ids(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|book| book["id"].as_str().expect("Book ID"))
        .collect()
}

/// Marks a book completed, then waits so that later reads are more recent.
async fn complete(server: &TestServer, book_id: &str) {
    mark(server, book_id, json!({"completed": true})).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
}

async fn reader_complete(server: &TestServer, reader: &Reader, book_id: &str) {
    let path = format!("/api/v1/books/{book_id}/read-progress");
    let request = reader
        .request(server, Method::PATCH, &path)
        .json(&json!({"completed": true}));
    assert_eq!(fetch(request).await.status, StatusCode::NO_CONTENT);
}

async fn ids_by_name(pool: &SqlitePool, names: &[&str]) -> Vec<String> {
    let mut ids = Vec::new();
    for name in names {
        ids.push(book(pool, name).await.0);
    }
    ids
}

#[tokio::test]
async fn the_next_unread_book_of_paused_series_comes_by_latest_read() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = [
        "Alpha/a1.cbz",
        "Alpha/a2.cbz",
        "Alpha/a3.cbz",
        "Beta/b1.cbz",
        "Beta/b2.cbz",
        "Gamma/g1.cbz",
        "Gamma/g2.cbz",
        "Delta/d1.cbz",
    ]
    .map(|path| (path, pages_book()));
    let (server, library, pool) = library_with_books(directory.path(), &books).await;
    let [a1, a2, a3, b1, b2, g1, g2, d1] = <[String; 8]>::try_from(
        ids_by_name(&pool, &["a1", "a2", "a3", "b1", "b2", "g1", "g2", "d1"]).await,
    )
    .expect("Eight books");
    assert_eq!(ids(&on_deck(&server, "").await), Vec::<&str>::new());

    complete(&server, &a1).await;
    complete(&server, &b1).await;
    mark(&server, &b2, json!({"page": 2})).await;
    complete(&server, &d1).await;
    complete(&server, &g1).await;

    // Beta has a book in progress and Delta is finished.
    let page = on_deck(&server, "").await;
    assert_eq!(ids(&page), [g2.as_str(), a2.as_str()]);
    assert_eq!(page["totalElements"], 2);
    assert_eq!(page["sort"]["sorted"], false);
    let first = &page["content"][0];
    assert_eq!(first["seriesTitle"], "Gamma");
    assert_eq!(first["name"], "g2");
    assert_eq!(first["media"]["mediaProfile"], "DIVINA");
    assert_eq!(first["readProgress"], Value::Null);
    let url = Path::new(first["url"].as_str().expect("Book URL"));
    assert!(
        url.is_absolute() && url.ends_with("Gamma/g2.cbz"),
        "{}",
        url.display()
    );

    let second_page = on_deck(&server, "?size=1&page=1&sort=name,desc").await;
    assert_eq!(ids(&second_page), [a2.as_str()]);
    assert_eq!(second_page["totalElements"], 2);
    let library_id = library["id"].as_str().expect("Library ID");
    let in_library = on_deck(&server, &format!("?library_id={library_id}")).await;
    assert_eq!(ids(&in_library), [g2.as_str(), a2.as_str()]);
    assert_eq!(
        ids(&on_deck(&server, "?library_id=0").await),
        Vec::<&str>::new()
    );

    // Reading moves Alpha to its next book and to the top.
    complete(&server, &a2).await;
    assert_eq!(ids(&on_deck(&server, "").await), [a3.as_str(), g2.as_str()]);
    server.stop().await;
}

#[tokio::test]
async fn each_user_sees_their_own_series_within_their_restrictions() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = [
        "Alpha/a1.cbz",
        "Alpha/a2.cbz",
        "Gamma/g1.cbz",
        "Gamma/g2.cbz",
    ]
    .map(|path| (path, pages_book()));
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let [a1, a2, g1, _] =
        <[String; 4]>::try_from(ids_by_name(&pool, &["a1", "a2", "g1", "g2"]).await)
            .expect("Four books");
    let gamma = book(&pool, "g1").await.1;
    let access = json!({"roles": [], "ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    let reader = create_reader(&server, READER, access).await;
    reader_complete(&server, &reader, &a1).await;
    reader_complete(&server, &reader, &g1).await;
    assert_eq!(ids(&on_deck(&server, "").await), Vec::<&str>::new());

    rate(&server, &gamma, 18).await;
    let response = fetch(reader.request(&server, Method::GET, ON_DECK)).await;
    assert_eq!(response.status, StatusCode::OK);
    let page: Value = serde_json::from_slice(&response.body).expect("JSON page");
    assert_eq!(ids(&page), [a2.as_str()]);
    // Only admins see the path of the file.
    assert_eq!(page["content"][0]["url"], "a2.cbz");

    let anonymous = fetch(server.request(Method::GET, ON_DECK)).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    server.stop().await;
}
