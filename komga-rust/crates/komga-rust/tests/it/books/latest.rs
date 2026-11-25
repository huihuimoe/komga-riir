//! Latest books (`BookController.getBooksLatest`).

use std::fs;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::reading::access::create_reader;
use super::reading::{fetch, pages_book};
use super::{JPEG, library_with_books};
use crate::libraries::route;
use crate::read_lists::rate;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::tasks::wait_for_scan_work;
use crate::support::{TestServer, openapi};

const LATEST: &str = "/api/v1/books/latest";

async fn latest(server: &TestServer, query: &str) -> Value {
    let response = server
        .admin(Method::GET, &format!("{LATEST}{query}"))
        .send()
        .await
        .expect("Latest request");
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::GET, response).await
}

fn names(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|book| book["name"].as_str().expect("Book name"))
        .collect()
}

pub(crate) async fn rescan(server: &TestServer, directory: &Path, library: &Value, query: &str) {
    let response = server
        .admin(Method::POST, &format!("{}/scan{query}", route(library)))
        .send()
        .await
        .expect("Scan request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    wait_for_scan_work(directory).await;
}

#[tokio::test]
async fn added_and_changed_books_come_first() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = ["Alpha/a1.cbz", "Beta/b1.cbz"].map(|path| (path, pages_book()));
    let (server, library, _) = library_with_books(directory.path(), &books).await;
    let root = directory.path().join("comics");
    let changed = cbz::zip(&[("p1.jpg", STORED, &cbz::fixture(JPEG))]);
    fs::write(root.join("Alpha/a1.cbz"), changed).expect("Changed book");
    // Rewriting a file keeps the directory time, so only a deep scan sees it; a deep scan also
    // resorts every existing series, whereas the follow-up hashing updates only the changed book.
    rescan(&server, directory.path(), &library, "?deep=true").await;
    fs::create_dir_all(root.join("Gamma")).expect("Series directory");
    fs::write(root.join("Gamma/g1.cbz"), pages_book()).expect("Added book");
    rescan(&server, directory.path(), &library, "").await;

    let page = latest(&server, "").await;
    assert_eq!(names(&page), ["g1", "a1", "b1"]);
    assert_eq!(page["totalElements"], 3);
    assert_eq!(page["sort"]["sorted"], true);
    // Sort parameters do not apply.
    assert_eq!(
        names(&latest(&server, "?sort=name,asc").await),
        ["g1", "a1", "b1"]
    );
    let second = latest(&server, "?size=1&page=1").await;
    assert_eq!(names(&second), ["a1"]);
    assert_eq!(second["totalElements"], 3);
    let unpaged = latest(&server, "?unpaged=true&size=1").await;
    assert_eq!(names(&unpaged), ["g1", "a1", "b1"]);
    assert_eq!(unpaged["size"], 20);
    server.stop().await;
}

#[tokio::test]
async fn readers_see_only_allowed_books_without_paths() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = ["Alpha/a1.cbz", "Gamma/g1.cbz"].map(|path| (path, pages_book()));
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    rate(&server, &book(&pool, "g1").await.1, 18).await;
    let access = json!({"roles": [], "ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    let reader = create_reader(&server, "reader@example.com", access).await;
    let response = fetch(reader.get(&server, LATEST)).await;
    assert_eq!(response.status, StatusCode::OK);
    let page: Value = serde_json::from_slice(&response.body).expect("JSON page");
    assert_eq!(names(&page), ["a1"]);
    assert_eq!(page["totalElements"], 1);
    // Only admins see the path of the file.
    assert_eq!(page["content"][0]["url"], "a1.cbz");

    let anonymous = fetch(server.request(Method::GET, LATEST)).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    server.stop().await;
}
