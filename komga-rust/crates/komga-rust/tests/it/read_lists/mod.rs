//! Read lists over real HTTP against scanned libraries.

mod books;
mod cleanup;
mod crud;
mod download;
mod list;
mod match_comicrack;
mod of_book;
mod search_index;
mod tachiyomi;
mod thumbnails;

use std::fs;
use std::path::Path;

use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::libraries::create;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::tasks::wait_for_scan_work;
use crate::support::{TestServer, db, openapi};

const READ_LISTS: &str = "/api/v1/readlists";
const PNG: &str = "komga-image/tests/fixtures/photo.png";
pub(crate) const PASSWORD: &str = "secret";

/// A claimed server over library `A` (series `SA`: `a1`, `a2`) and library `B` (series `SB`:
/// `b1`), with the IDs of what it scanned.
pub(crate) struct Fixture {
    pub(crate) server: TestServer,
    pub(crate) pool: SqlitePool,
    pub(crate) library_a: String,
    pub(crate) a1: String,
    pub(crate) a2: String,
    pub(crate) b1: String,
    pub(crate) series_a: String,
    pub(crate) series_b: String,
}

pub(crate) async fn fixture(directory: &Path) -> Fixture {
    let png = cbz::fixture(PNG);
    let file = cbz::zip(&[("page.png", STORED, &png)]);
    for path in ["a/SA/a1.cbz", "a/SA/a2.cbz", "b/SB/b1.cbz"] {
        let path = directory.join(path);
        fs::create_dir_all(path.parent().expect("Series directory")).expect("Series directory");
        fs::write(path, &file).expect("Book file");
    }
    let server = TestServer::start_claimed(directory).await;
    let library_a = create(&server, "A", &directory.join("a")).await;
    create(&server, "B", &directory.join("b")).await;
    wait_for_scan_work(directory).await;
    let pool = db::main_pool(directory).await;
    let (a1, series_a) = book(&pool, "a1").await;
    let (a2, _) = book(&pool, "a2").await;
    let (b1, series_b) = book(&pool, "b1").await;
    Fixture {
        server,
        pool,
        library_a: library_a["id"].as_str().expect("Library ID").to_owned(),
        a1,
        a2,
        b1,
        series_a,
        series_b,
    }
}

/// Creates read list `name` of `books` in order; its ID.
pub(crate) async fn create_list(fixture: &Fixture, name: &str, books: &[&str]) -> String {
    let body = json!({"name": name, "bookIds": books});
    let response = send(&fixture.server, None, Method::POST, READ_LISTS, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let created: Value = openapi::checked(Method::POST, response).await;
    created["id"].as_str().expect("Read list ID").to_owned()
}

/// Creates a user with the given access fields; it signs in with [`PASSWORD`].
pub(crate) async fn create_user(server: &TestServer, email: &str, access: Value) {
    let mut body = access;
    body["email"] = json!(email);
    body["password"] = json!(PASSWORD);
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&body)
        .send()
        .await
        .expect("User creation");
    assert_eq!(response.status(), StatusCode::CREATED, "{email}");
}

/// A request as `email`, or as the claimed admin without one.
pub(crate) async fn send(
    server: &TestServer,
    email: Option<&str>,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Response {
    let mut request = match email {
        Some(email) => server
            .request(method, path)
            .basic_auth(email, Some(PASSWORD)),
        None => server.admin(method, path),
    };
    if let Some(body) = body {
        request = request.json(body);
    }
    request.send().await.expect("Read list request")
}

/// Sets the age rating of a series through its metadata edit.
pub(crate) async fn rate(server: &TestServer, series_id: &str, age_rating: i32) {
    let path = format!("/api/v1/series/{series_id}/metadata");
    let body = json!({"ageRating": age_rating});
    let response = send(server, None, Method::PATCH, &path, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

/// `(BOOK_COUNT, BOOK_ID by NUMBER)` of the read list `id`.
async fn stored_members(pool: &SqlitePool, id: &str) -> (i64, Vec<(i64, String)>) {
    let count = sqlx::query_scalar("SELECT BOOK_COUNT FROM READLIST WHERE ID = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("Read list count");
    let members = sqlx::query_as(
        "SELECT NUMBER, BOOK_ID FROM READLIST_BOOK WHERE READLIST_ID = ? ORDER BY NUMBER",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .expect("Read list members");
    (count, members)
}
