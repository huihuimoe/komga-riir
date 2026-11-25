//! The KOReader sync server (`KoreaderSyncController`): books found by their KOReader hash.

use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::books::PNG;
use crate::books::epub::{divina_book, sample};
use crate::books::library_with_books;
use crate::books::read_progress::progress;
use crate::books::reading::access::{Reader, create_reader};
use crate::books::reading::pages_book;
use crate::kobo::api_key;
use crate::support::cbz;
use crate::support::db::content::book;
use crate::support::tasks::{TaskRow, wait_for_tasks};
use crate::support::{ADMIN_EMAIL, TestServer, json};

const PROGRESS: &str = "/koreader/syncs/progress";

/// A library of a three-page CBZ `pages`, the EPUB novel `novel`, two copies of one DiViNa
/// EPUB and the unreadable `broken`, all hashed for KOReader.
async fn hashed_library(directory: &Path) -> (TestServer, SqlitePool) {
    let copy = divina_book(&cbz::fixture(PNG));
    let books = [
        ("Pages/pages.cbz", pages_book()),
        (
            "Novel/novel.epub",
            sample("The Incomplete Theft - Ralph Burke.epub"),
        ),
        ("Copies/first.epub", copy.clone()),
        ("Copies/second.epub", copy),
        ("Broken/broken.cbz", b"not an archive".to_vec()),
    ];
    let (server, library, pool) = library_with_books(directory, &books).await;
    let path = format!(
        "/api/v1/libraries/{}",
        library["id"].as_str().expect("Library id")
    );
    let response = server
        .admin(Method::PATCH, &path)
        .json(&json!({"hashKoreader": true}))
        .send()
        .await
        .expect("Library update");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    wait_for_tasks(directory, <[TaskRow]>::is_empty).await;
    (server, pool)
}

async fn hash(pool: &SqlitePool, name: &str) -> String {
    let hash: String = sqlx::query_scalar("SELECT FILE_HASH_KOREADER FROM BOOK WHERE NAME = ?")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("KOReader hash");
    assert_eq!(hash.len(), 32, "{name} is hashed");
    hash
}

fn update(document: &str, percentage: f32, progress: &str) -> Value {
    json!({
        "document": document,
        "percentage": percentage,
        "progress": progress,
        "device": "Kindle",
        "device_id": "kindle-1",
    })
}

async fn send(
    server: &TestServer,
    key: &str,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    let mut request = server.request(method, path).header("X-Auth-User", key);
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("KOReader request");
    let status = response.status();
    let text = response.text().await.expect("KOReader body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn get(server: &TestServer, key: &str, hash: &str) -> (StatusCode, Value) {
    send(
        server,
        key,
        Method::GET,
        &format!("{PROGRESS}/{hash}"),
        None,
    )
    .await
}

async fn put(server: &TestServer, key: &str, body: &Value) -> StatusCode {
    send(server, key, Method::PUT, PROGRESS, Some(body)).await.0
}

#[tokio::test]
async fn koreader_requests_need_a_header_key_and_the_koreader_role() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = [("Pages/pages.cbz", pages_book())];
    let (server, _, _) = library_with_books(directory.path(), &books).await;
    let auth = "/koreader/users/auth";
    let reader = create_reader(&server, "kobo@example.org", json!({"roles": ["KOBO_SYNC"]})).await;
    let other_role = api_key(&server, Some(&reader)).await;
    let key = api_key(&server, None).await;

    let anonymous = server
        .request(Method::GET, auth)
        .send()
        .await
        .expect("Auth");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        get(&server, "unknown", "hash").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&server, &other_role, Method::GET, auth, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&server, &key, Method::GET, auth, None).await,
        (StatusCode::OK, json!({"authorized": "OK"}))
    );

    // Registration is refused to everyone, signed in or not.
    let create = "/koreader/users/create";
    let body = json!({"username": "new", "password": "secret"});
    let anonymous = server
        .request(Method::POST, create)
        .json(&body)
        .send()
        .await;
    let anonymous = anonymous.expect("Registration");
    assert_eq!(anonymous.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        json(anonymous).await["message"],
        "User creation is disabled"
    );
    let (status, _) = send(&server, &key, Method::POST, create, Some(&body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn page_progress_round_trips_through_the_hash() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = hashed_library(directory.path()).await;
    let key = api_key(&server, None).await;
    let pages = hash(&pool, "pages").await;
    let book_id = book(&pool, "pages").await.0;

    assert_eq!(
        get(&server, &key, "0123456789abcdef").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&server, &key, &pages).await,
        (StatusCode::OK, json!({})),
        "no progress yet"
    );
    assert_eq!(
        put(&server, &key, &update(&pages, 0.5, "2")).await,
        StatusCode::OK
    );
    // A retried update lands again.
    assert_eq!(
        put(&server, &key, &update(&pages, 0.5, "2")).await,
        StatusCode::OK
    );
    let stored = (
        ADMIN_EMAIL.to_owned(),
        2,
        false,
        "kindle-1".to_owned(),
        "Kindle".to_owned(),
    );
    assert_eq!(
        progress(&pool, &book_id).await,
        std::slice::from_ref(&stored)
    );
    let (status, mut body) = get(&server, &key, &pages).await;
    assert_eq!(status, StatusCode::OK);
    // Page progress keeps the page alone: 2 of 3.
    let percentage = body["percentage"].take().as_f64().expect("Percentage");
    assert!((percentage - 2.0 / 3.0).abs() < 1e-6, "{percentage}");
    let expected = json!({
        "document": pages,
        "percentage": null,
        "progress": "2",
        "device": "Kindle",
        "device_id": "kindle-1",
    });
    assert_eq!(body, expected);

    for wrong in ["two", "0", "4"] {
        let status = put(&server, &key, &update(&pages, 0.5, wrong)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong}");
    }
    let missing = put(&server, &key, &update("0123456789abcdef", 0.5, "1")).await;
    assert_eq!(missing, StatusCode::NOT_FOUND);
    assert_eq!(progress(&pool, &book_id).await, [stored]);
}

#[tokio::test]
async fn duplicated_hashes_are_conflicts() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = hashed_library(directory.path()).await;
    let key = api_key(&server, None).await;
    let copies = hash(&pool, "first").await;
    assert_eq!(copies, hash(&pool, "second").await);

    let (status, body) = get(&server, &key, &copies).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["message"], "More than 1 book found with the same hash");
    let status = put(&server, &key, &update(&copies, 0.5, "1")).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn progress_of_a_book_without_a_media_profile_is_refused() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = hashed_library(directory.path()).await;
    let key = api_key(&server, None).await;
    let broken = hash(&pool, "broken").await;

    let status = put(&server, &key, &update(&broken, 0.5, "1")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// The distinct resources of the novel's positions, with the total progression each starts at.
async fn resources(server: &TestServer, book_id: &str) -> Vec<(String, f64)> {
    let path = format!("/api/v1/books/{book_id}/positions");
    let response = server
        .admin(Method::GET, &path)
        .send()
        .await
        .expect("Positions");
    let mut resources: Vec<(String, f64)> = Vec::new();
    for position in json(response).await["positions"]
        .as_array()
        .expect("Positions")
    {
        let href = position["href"].as_str().expect("Href").to_owned();
        if resources.iter().all(|(seen, _)| *seen != href) {
            let total = position["locations"]["totalProgression"]
                .as_f64()
                .expect("Total");
            resources.push((href, total));
        }
    }
    resources
}

#[tokio::test]
async fn epub_progress_points_to_resources() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = hashed_library(directory.path()).await;
    let key = api_key(&server, None).await;
    let novel = hash(&pool, "novel").await;
    let resources = resources(&server, &book(&pool, "novel").await.0).await;
    assert!(resources.len() >= 3);
    let percentage = |body: &Value| body["percentage"].as_f64().expect("Percentage");

    let xpointer = "/body/DocFragment[2]/body/div/p[1]/text().0";
    assert_eq!(
        put(&server, &key, &update(&novel, 0.9, xpointer)).await,
        StatusCode::OK
    );
    let (_, body) = get(&server, &key, &novel).await;
    assert_eq!(body["progress"], "/body/DocFragment[2].0");
    // The analyzed start of the resource, not what KOReader reported.
    assert!((percentage(&body) - resources[1].1).abs() < 1e-6);

    let anchor = "#_doc_fragment_2_ c37";
    assert_eq!(
        put(&server, &key, &update(&novel, 0.1, anchor)).await,
        StatusCode::OK
    );
    assert_eq!(
        get(&server, &key, &novel).await.1["progress"],
        "/body/DocFragment[3].0"
    );

    for wrong in ["/body/DocFragment[999].0", "/body/DocFragment[0].0", "12"] {
        let status = put(&server, &key, &update(&novel, 0.1, wrong)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong}");
    }

    // A progression from another reader maps back to its resource.
    let book_id = book(&pool, "novel").await.0;
    let progression = json!({
        "modified": "2999-01-01T00:00:00Z",
        "device": {"id": "web", "name": "Web"},
        "locator": {
            "href": format!("{}#part", resources[0].0),
            "type": "application/xhtml+xml",
            "locations": {"progression": 0},
        },
    });
    let path = format!("/api/v1/books/{book_id}/progression");
    let response = server
        .admin(Method::PUT, &path)
        .json(&progression)
        .send()
        .await;
    assert_eq!(
        response.expect("Progression").status(),
        StatusCode::NO_CONTENT
    );
    let (_, body) = get(&server, &key, &novel).await;
    assert_eq!(body["progress"], "/body/DocFragment[1].0");
    assert_eq!(
        (body["device"].clone(), body["device_id"].clone()),
        (json!("Web"), json!("web"))
    );
}

async fn reader_key(server: &TestServer, email: &str, access: Value) -> (Reader, String) {
    let mut access = access;
    access["roles"] = json!(["KOREADER_SYNC"]);
    let reader = create_reader(server, email, access).await;
    let key = api_key(server, Some(&reader)).await;
    (reader, key)
}

#[tokio::test]
async fn users_only_reach_their_own_progress_on_books_they_see() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = hashed_library(directory.path()).await;
    let admin = api_key(&server, None).await;
    let pages = hash(&pool, "pages").await;
    let book_id = book(&pool, "pages").await.0;
    let (_, shared) = reader_key(&server, "shared@example.org", json!({})).await;
    let none = json!({"sharedLibraries": {"all": false, "libraryIds": []}});
    let (_, outsider) = reader_key(&server, "outsider@example.org", none).await;

    assert_eq!(
        put(&server, &admin, &update(&pages, 0.5, "2")).await,
        StatusCode::OK
    );
    assert_eq!(
        get(&server, &shared, &pages).await,
        (StatusCode::OK, json!({}))
    );
    assert_eq!(
        put(&server, &shared, &update(&pages, 1.0, "3")).await,
        StatusCode::OK
    );
    assert_eq!(get(&server, &admin, &pages).await.1["progress"], "2");
    assert_eq!(get(&server, &shared, &pages).await.1["progress"], "3");

    // Books of libraries a user cannot see are not found, for reading and writing.
    assert_eq!(
        get(&server, &outsider, &pages).await.0,
        StatusCode::NOT_FOUND
    );
    let status = put(&server, &outsider, &update(&pages, 0.5, "1")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let emails: Vec<_> = progress(&pool, &book_id)
        .await
        .into_iter()
        .map(|row| row.0)
        .collect();
    assert_eq!(emails, [ADMIN_EMAIL, "shared@example.org"]);
}
