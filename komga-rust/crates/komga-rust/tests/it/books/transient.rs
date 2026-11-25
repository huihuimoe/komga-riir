//! Transient books: books outside every library, reviewed before an import.

use std::fs;
use std::path::Path;

use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::reading::fetch;
use super::{JPEG, PNG, library_with_books};
use crate::support::cbz::{self, STORED};
use crate::support::{TestServer, json, openapi};

const ROUTE: &str = "/api/v1/transient-books";

fn issue(series: &str, number: &str) -> Vec<u8> {
    let comic_info =
        format!("<ComicInfo><Series>{series}</Series><Number>{number}</Number></ComicInfo>");
    cbz::zip(&[
        ("1.jpg", STORED, &cbz::fixture(JPEG)),
        ("2.png", STORED, &cbz::fixture(PNG)),
        ("ComicInfo.xml", STORED, comic_info.as_bytes()),
    ])
}

/// A server whose library holds the series `Watchmen`, and a folder outside it holding
/// `Watchmen 2.cbz`.
async fn server_with_incoming(directory: &Path) -> (TestServer, SqlitePool, std::path::PathBuf) {
    let (server, _, pool) =
        library_with_books(directory, &[("Watchmen/1.cbz", issue("Watchmen", "1"))]).await;
    let incoming = directory.join("incoming");
    fs::create_dir(&incoming).expect("Incoming folder");
    fs::write(incoming.join("Watchmen 2.cbz"), issue("Watchmen", "2")).expect("Incoming book");
    (server, pool, incoming)
}

async fn scan(server: &TestServer, path: &Path) -> reqwest::Response {
    server
        .admin(Method::POST, ROUTE)
        .json(&json!({"path": path}))
        .send()
        .await
        .expect("Scan request")
}

async fn stored_books(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM BOOK")
        .fetch_one(pool)
        .await
        .expect("Book count")
}

#[tokio::test]
async fn books_are_scanned_analyzed_and_read_without_touching_the_library() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool, incoming) = server_with_incoming(directory.path()).await;
    let response = scan(&server, &incoming).await;
    assert_eq!(response.status(), StatusCode::OK);
    let books = openapi::checked(Method::POST, response).await;
    let file = incoming.join("Watchmen 2.cbz");
    assert_eq!(books[0]["name"], "Watchmen 2");
    assert_eq!(books[0]["url"], json!(file));
    assert_eq!(books[0]["status"], "UNKNOWN");
    assert_eq!(books[0]["pages"], json!([]));
    assert_eq!(books.as_array().map(Vec::len), Some(1));
    let id = books[0]["id"].as_str().expect("Transient book id");

    let page = format!("{ROUTE}/{id}/pages/1");
    let unanalyzed = fetch(server.admin(Method::GET, &page)).await;
    assert_eq!(unanalyzed.status, StatusCode::NOT_FOUND);
    assert_eq!(unanalyzed.message(), "Book analysis failed");

    let analyze = format!("{ROUTE}/{id}/analyze");
    let response = server
        .admin(Method::POST, &analyze)
        .send()
        .await
        .expect("Analyze request");
    assert_eq!(response.status(), StatusCode::OK);
    let book = openapi::checked(Method::POST, response).await;
    let (_, series_id) = crate::support::db::content::book(&pool, "1").await;
    assert_eq!(book["status"], "READY");
    assert_eq!(book["mediaType"], "application/zip");
    assert_eq!(book["number"], 2.0);
    assert_eq!(book["seriesId"], series_id);
    assert_eq!(book["pages"][1]["fileName"], "2.png");
    assert_eq!(book["files"], json!(["ComicInfo.xml"]));

    let read = fetch(server.admin(Method::GET, &page)).await;
    assert_eq!(read.status, StatusCode::OK);
    assert_eq!(read.header(&CONTENT_TYPE), "image/jpeg");
    assert_eq!(read.body, cbz::fixture(JPEG));
    let beyond = fetch(server.admin(Method::GET, &format!("{ROUTE}/{id}/pages/3"))).await;
    assert_eq!(beyond.status, StatusCode::BAD_REQUEST);
    assert_eq!(beyond.message(), "Page number does not exist");

    assert!(file.exists(), "Transient books leave their files");
    assert_eq!(stored_books(&pool).await, 1);
    server.stop().await;
}

#[tokio::test]
async fn folders_of_libraries_and_unreadable_folders_are_refused() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _pool, incoming) = server_with_incoming(directory.path()).await;
    let library = directory.path().join("comics");
    #[cfg(unix)]
    let alias = {
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&library, &alias).expect("Library alias");
        alias
    };
    for (path, message) in [
        (library.join("Watchmen"), "ERR_1017"),
        #[cfg(unix)]
        (alias.join("Watchmen"), "ERR_1017"),
        (directory.path().join("missing"), "ERR_1016"),
        (Path::new("incoming").to_path_buf(), "ERR_1016"),
    ] {
        let response = scan(&server, &path).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{}",
            path.display()
        );
        assert_eq!(
            json(response).await["message"],
            message,
            "{}",
            path.display()
        );
    }
    let unknown =
        fetch(server.admin(Method::POST, &format!("{ROUTE}/0AAAAAAAAAAAA/analyze"))).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&json!({"email":"reader@example.com", "password":"readerpass"}))
        .send()
        .await
        .expect("Create reader");
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = server
        .request(Method::POST, ROUTE)
        .basic_auth("reader@example.com", Some("readerpass"))
        .json(&json!({"path": incoming}))
        .send()
        .await
        .expect("Reader scan");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let _: Value = json(response).await;
    server.stop().await;
}
