//! Who may read which book, and books that cannot be read.

use std::fs;

use reqwest::{Method, RequestBuilder, StatusCode};
use serde_json::{Value, json};

use super::{fetch, pages_book};
use crate::books::{PNG, library_with_books};
use crate::support::TestServer;
use crate::support::cbz::{self, ZSTD};
use crate::support::db::content::book;
use crate::support::db::seed;

pub(crate) struct Reader {
    email: String,
}

impl Reader {
    pub(crate) fn get(&self, server: &TestServer, path: &str) -> RequestBuilder {
        self.request(server, Method::GET, path)
    }

    pub(crate) fn request(
        &self,
        server: &TestServer,
        method: Method,
        path: &str,
    ) -> RequestBuilder {
        server
            .request(method, path)
            .basic_auth(&self.email, Some("secret"))
    }
}

/// A user created with `access` (roles, shared libraries, restrictions).
pub(crate) async fn create_reader(server: &TestServer, email: &str, access: Value) -> Reader {
    let mut body = access;
    body["email"] = json!(email);
    body["password"] = json!("secret");
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&body)
        .send()
        .await
        .expect("User creation");
    assert_eq!(response.status(), StatusCode::CREATED, "{email}");
    Reader {
        email: email.to_owned(),
    }
}

/// Access with `role` and the `extra` fields (shared libraries, restrictions).
pub(crate) fn with_role(role: &str, extra: Value) -> Value {
    let mut access = json!({"roles": [role]});
    access
        .as_object_mut()
        .expect("Access object")
        .extend(extra.as_object().expect("Extra access").clone());
    access
}

#[tokio::test]
async fn pages_need_page_streaming_and_visible_content() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/book.cbz", pages_book())]).await;
    let (book_id, series_id) = book(&pool, "book").await;
    seed::series_age_rating(&pool, &series_id, 16).await;
    let pages = format!("/api/v1/books/{book_id}/pages");
    let page = format!("{pages}/1");
    let raw = format!("{pages}/1/raw");
    let streaming = |extra| with_role("PAGE_STREAMING", extra);

    let unauthenticated = fetch(server.request(Method::GET, &pages)).await;
    assert_eq!(unauthenticated.status, StatusCode::UNAUTHORIZED);

    // Listing needs no role; reading does, checked after binding and before the book lookup.
    let no_role = create_reader(&server, "no-role@example.com", json!({})).await;
    let missing = "/api/v1/books/missing/pages/1";
    for (path, status, message) in [
        (pages.as_str(), StatusCode::OK, None),
        (page.as_str(), StatusCode::FORBIDDEN, Some("Forbidden")),
        (raw.as_str(), StatusCode::FORBIDDEN, Some("Forbidden")),
        (missing, StatusCode::FORBIDDEN, Some("Forbidden")),
    ] {
        let response = fetch(no_role.get(&server, path)).await;
        assert_eq!(response.status, status, "{path}");
        if let Some(message) = message {
            assert_eq!(response.message(), message, "{path}");
        }
    }
    let unbound = fetch(no_role.get(&server, &format!("{pages}/one"))).await;
    assert_eq!(unbound.status, StatusCode::BAD_REQUEST);

    let denied = [
        streaming(json!({"sharedLibraries": {"all": false, "libraryIds": []}})),
        streaming(json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}})),
        streaming(json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}})),
    ];
    for (index, access) in denied.into_iter().enumerate() {
        let reader = create_reader(&server, &format!("denied-{index}@example.com"), access).await;
        for path in [&pages, &page, &raw] {
            let response = fetch(reader.get(&server, path)).await;
            assert_eq!(response.status, StatusCode::FORBIDDEN, "{index} {path}");
            assert_eq!(response.message(), "403 FORBIDDEN", "{index} {path}");
        }
    }
    let allowed = streaming(json!({"ageRestriction": {"age": 16, "restriction": "ALLOW_ONLY"}}));
    let reader = create_reader(&server, "allowed@example.com", allowed).await;
    for path in [&pages, &page] {
        assert_eq!(
            fetch(reader.get(&server, path)).await.status,
            StatusCode::OK,
            "{path}"
        );
    }

    for path in [
        "/api/v1/books/missing/pages",
        missing,
        "/api/v1/books/missing/pages/1/raw",
    ] {
        let response = fetch(server.admin(Method::GET, path)).await;
        assert_eq!(response.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(response.message(), "404 NOT_FOUND", "{path}");
    }
    server.stop().await;
}

#[tokio::test]
async fn unready_and_missing_books_cannot_be_read() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let zstd = cbz::zip(&[("page.png", ZSTD, &cbz::fixture(PNG))]);
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("text.cbz", b"plain text, not a book".to_vec()),
            ("broken.cbz", b"PK\x03\x04 is not a whole archive".to_vec()),
            ("zstd.cbz", zstd),
            ("outdated.cbz", pages_book()),
            ("moved.cbz", pages_book()),
        ],
    )
    .await;
    let (outdated, _) = book(&pool, "outdated").await;
    seed::media_status(&pool, &outdated, "OUTDATED").await;

    for (name, message) in [
        ("text", "Book format is not supported"),
        ("broken", "Book analysis failed"),
        ("zstd", "Book has not been analyzed yet"),
        ("outdated", "Book is outdated and must be re-analyzed"),
    ] {
        let (id, _) = book(&pool, name).await;
        let pages = format!("/api/v1/books/{id}/pages");
        let listed = fetch(server.admin(Method::GET, &pages)).await;
        assert_eq!(listed.status, StatusCode::NOT_FOUND, "{name}");
        assert_eq!(listed.message(), message, "{name}");
        let page = fetch(server.admin(Method::GET, &format!("{pages}/1"))).await;
        assert_eq!(page.status, StatusCode::NOT_FOUND, "{name}");
        assert_eq!(page.message(), "Book analysis failed", "{name}");
    }

    // The media still lists pages of a moved file, which can no longer be read.
    let (moved, _) = book(&pool, "moved").await;
    fs::remove_file(directory.path().join("comics/moved.cbz")).expect("Move book away");
    let pages = format!("/api/v1/books/{moved}/pages");
    assert_eq!(
        fetch(server.admin(Method::GET, &pages)).await.status,
        StatusCode::OK
    );
    let page = fetch(server.admin(Method::GET, &format!("{pages}/1"))).await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);
    assert_eq!(page.message(), "File not found, it may have moved");
    server.stop().await;
}
