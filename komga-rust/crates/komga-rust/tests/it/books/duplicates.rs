//! Duplicate books (`BookController.getBooksDuplicates`).

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::reading::access::create_reader;
use super::reading::{fetch, pages_book};
use super::{JPEG, library_with_books};
use crate::read_lists::rate;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

const DUPLICATES: &str = "/api/v1/books/duplicates";

async fn duplicates(server: &TestServer, query: &str) -> Value {
    let response = server
        .admin(Method::GET, &format!("{DUPLICATES}{query}"))
        .send()
        .await
        .expect("Duplicates request");
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::GET, response).await
}

fn in_order(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|book| book["name"].as_str().expect("Book name"))
        .collect()
}

/// Books of one hash come in ID order, which follows the scan.
fn names(page: &Value) -> Vec<&str> {
    let mut names = in_order(page);
    names.sort_unstable();
    names
}

#[tokio::test]
async fn books_with_the_same_file_are_listed_for_admins() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let unique = cbz::zip(&[("p1.jpg", STORED, &cbz::fixture(JPEG))]);
    let books = [
        ("Alpha/a1.cbz", pages_book()),
        ("Alpha/a2.cbz", pages_book()),
        ("Beta/b1.cbz", pages_book()),
        ("Beta/b2.cbz", unique),
    ];
    let (server, _, pool) = library_with_books(directory.path(), &books).await;

    let page = duplicates(&server, "").await;
    assert_eq!(names(&page), ["a1", "a2", "b1"]);
    assert_eq!(page["totalElements"], 3);
    assert_eq!(page["sort"]["sorted"], true);
    let hash = page["content"][0]["fileHash"].as_str().expect("File hash");
    assert!(!hash.is_empty());
    let by_name = duplicates(&server, "?sort=name,desc").await;
    assert_eq!(in_order(&by_name), ["b1", "a2", "a1"]);
    assert_eq!(
        duplicates(&server, "?sort=unknown").await["sort"]["sorted"],
        false
    );
    let last = duplicates(&server, "?size=2&page=1").await;
    assert_eq!(last["content"].as_array().map(Vec::len), Some(1));
    assert_eq!(last["totalElements"], 3);
    assert_eq!(names(&duplicates(&server, "?unpaged=true").await).len(), 3);

    // Content restrictions apply to admins too.
    rate(&server, &book(&pool, "b1").await.1, 18).await;
    let access =
        json!({"roles": ["ADMIN"], "ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    let restricted = create_reader(&server, "admin2@example.com", access).await;
    let response = fetch(restricted.get(&server, DUPLICATES)).await;
    assert_eq!(response.status, StatusCode::OK);
    let page: Value = serde_json::from_slice(&response.body).expect("JSON page");
    assert_eq!(names(&page), ["a1", "a2"]);
    assert_eq!(page["totalElements"], 2);
    server.stop().await;
}

#[tokio::test]
async fn duplicates_are_for_admins_only() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let reader = create_reader(&server, "reader@example.com", json!({"roles": []})).await;
    let response = fetch(reader.get(&server, DUPLICATES)).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    let anonymous = fetch(server.request(Method::GET, DUPLICATES)).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    server.stop().await;
}
