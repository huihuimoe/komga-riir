//! Duplicate pages by hash: unknown hashes, their pages, and hashes marked with an action.

use std::path::Path;

use image::GenericImageView;
use komga_archive::digest::FileDigest;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::reading::access::create_reader;
use super::reading::{Fetched, fetch};
use super::thumbnails::cover;
use super::{PNG, library_with_books};
use crate::libraries::{admin_json, route};
use crate::support::TestServer;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::tasks::wait_for_scan_work;

const PAGE_HASHES: &str = "/api/v1/page-hashes";

fn file_hash(bytes: &[u8]) -> String {
    let mut digest = FileDigest::new();
    digest.update(bytes);
    digest.finish()
}

/// Three pages share the 800 x 400 cover and two the photo; the books are hashed.
async fn hashed_books(directory: &Path) -> (TestServer, Vec<String>) {
    let (cover, photo) = (cover(), cbz::fixture(PNG));
    let books = [
        (
            "a.cbz",
            cbz::zip(&[("1.png", STORED, &cover), ("2.png", STORED, &photo)]),
        ),
        (
            "b.cbz",
            cbz::zip(&[("1.png", STORED, &cover), ("2.png", STORED, &cover)]),
        ),
        ("c.cbz", cbz::zip(&[("1.png", STORED, &photo)])),
    ];
    let (server, library, pool) = library_with_books(directory, &books).await;
    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"hashPages": true})),
        StatusCode::NO_CONTENT,
    )
    .await;
    wait_for_scan_work(directory).await;
    let mut ids = Vec::new();
    for name in ["a", "b", "c"] {
        ids.push(book(&pool, name).await.0);
    }
    (server, ids)
}

async fn get(server: &TestServer, path: &str) -> Value {
    admin_json(server, Method::GET, path, None, StatusCode::OK).await
}

fn hashes(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|item| item["hash"].as_str().expect("Hash"))
        .collect()
}

/// Known hashes; unlike the OpenAPI document, a size may be null, as Kotlin sends it.
async fn known(server: &TestServer, query: &str) -> Value {
    let response = fetch(server.admin(Method::GET, &format!("{PAGE_HASHES}{query}"))).await;
    assert_eq!(response.status, StatusCode::OK, "{query}");
    serde_json::from_slice(&response.body).expect("Known hashes")
}

async fn put_known(server: &TestServer, body: &Value, status: StatusCode) -> Value {
    admin_json(server, Method::PUT, PAGE_HASHES, Some(body), status).await
}

#[tokio::test]
async fn unknown_duplicates_list_their_pages_and_show_the_first() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, ids) = hashed_books(directory.path()).await;
    let (cover, photo) = (cover(), cbz::fixture(PNG));
    let (cover_hash, photo_hash) = (file_hash(&cover), file_hash(&photo));

    let unknown = get(
        &server,
        &format!("{PAGE_HASHES}/unknown?sort=matchCount,desc"),
    )
    .await;
    assert_eq!(
        unknown["content"],
        json!([
            {"hash": cover_hash, "size": cover.len(), "matchCount": 3},
            {"hash": photo_hash, "size": photo.len(), "matchCount": 2},
        ])
    );
    assert_eq!(unknown["sort"]["sorted"], true);
    let paged = get(
        &server,
        &format!("{PAGE_HASHES}/unknown?size=1&page=1&sort=totalSize"),
    )
    .await;
    let largest = if 3 * cover.len() > 2 * photo.len() {
        &cover_hash
    } else {
        &photo_hash
    };
    assert_eq!(
        hashes(&paged),
        [largest.as_str()],
        "the larger total comes last"
    );
    assert_eq!(
        (
            paged["totalElements"].as_u64(),
            paged["totalPages"].as_u64()
        ),
        (Some(2), Some(2))
    );
    let unsorted = get(&server, &format!("{PAGE_HASHES}/unknown?sort=nothing,desc")).await;
    assert_eq!(
        unsorted["sort"]["sorted"], false,
        "unknown properties are dropped"
    );

    let matches = get(
        &server,
        &format!("{PAGE_HASHES}/{cover_hash}?sort=url&sort=pageNumber,desc"),
    )
    .await;
    let root = directory.path().join("comics");
    let page = |id: &String, file: &str, number: u32| {
        json!({
            "bookId": id, "url": root.join(file), "pageNumber": number, "fileName": format!("{number}.png"),
            "fileSize": cover.len(), "mediaType": "image/png",
        })
    };
    let expected = [
        page(&ids[0], "a.cbz", 1),
        page(&ids[1], "b.cbz", 2),
        page(&ids[1], "b.cbz", 1),
    ];
    assert_eq!(matches["content"], json!(expected));
    let unsortable = get(
        &server,
        &format!("{PAGE_HASHES}/{cover_hash}?sort=matchCount"),
    )
    .await;
    assert_eq!(
        unsortable["sort"]["sorted"], false,
        "matches have no count to sort by"
    );

    let thumbnail = format!("{PAGE_HASHES}/unknown/{cover_hash}/thumbnail");
    let stored = fetch(server.admin(Method::GET, &thumbnail)).await;
    assert_eq!(
        (stored.status, stored.header(&CONTENT_TYPE)),
        (StatusCode::OK, "image/png")
    );
    assert_eq!(stored.body, cover);
    let resized = fetch(server.admin(Method::GET, &format!("{thumbnail}?resize=100"))).await;
    assert_eq!(resized.header(&CONTENT_TYPE), "image/jpeg");
    let image = image::load_from_memory(&resized.body).expect("JPEG thumbnail");
    assert_eq!(image.dimensions(), (100, 50));
    assert_thumbnail_failures(&server, &thumbnail).await;
}

async fn assert_thumbnail_failures(server: &TestServer, thumbnail: &str) {
    for size in ["0", "-1", "x"] {
        let invalid = fetch(server.admin(Method::GET, &format!("{thumbnail}?resize={size}"))).await;
        assert_eq!(invalid.status, StatusCode::BAD_REQUEST, "{size}");
        assert_eq!(
            invalid.message(),
            format!("Invalid value for parameter 'resize': {size}")
        );
    }
    let missing = format!("{PAGE_HASHES}/unknown/missing/thumbnail");
    let missing: Fetched = fetch(server.admin(Method::GET, &missing)).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn marking_a_hash_known_stores_a_thumbnail_and_later_only_changes_the_action() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _) = hashed_books(directory.path()).await;
    let (cover_hash, photo_hash) = (file_hash(&cover()), file_hash(&cbz::fixture(PNG)));

    put_known(
        &server,
        &json!({"hash": cover_hash, "size": -1, "action": "IGNORE"}),
        StatusCode::ACCEPTED,
    )
    .await;
    let listed = known(&server, "").await;
    let item = &listed["content"][0];
    assert_eq!(
        (
            &item["hash"],
            &item["size"],
            &item["action"],
            &item["deleteCount"],
            &item["matchCount"]
        ),
        (
            &json!(cover_hash),
            &Value::Null,
            &json!("IGNORE"),
            &json!(0),
            &json!(3)
        ),
        "a negative size is unknown"
    );
    let created = item["created"].as_str().expect("Created").to_owned();
    assert!(
        jiff::civil::DateTime::strptime("%Y-%m-%dT%H:%M:%S", &created).is_ok(),
        "{created}"
    );
    assert_eq!(
        hashes(&get(&server, &format!("{PAGE_HASHES}/unknown")).await),
        [photo_hash.as_str()]
    );

    let thumbnail = fetch(server.admin(
        Method::GET,
        &format!("{PAGE_HASHES}/{cover_hash}/thumbnail"),
    ))
    .await;
    assert_eq!(thumbnail.header(&CONTENT_TYPE), "image/jpeg");
    let image = image::load_from_memory(&thumbnail.body).expect("Stored JPEG");
    assert_eq!(image.dimensions(), (500, 250));

    put_known(
        &server,
        &json!({"hash": cover_hash, "size": 5, "action": "DELETE_MANUAL"}),
        StatusCode::ACCEPTED,
    )
    .await;
    let item = known(&server, "").await["content"][0].clone();
    assert_eq!(
        (&item["action"], &item["size"]),
        (&json!("DELETE_MANUAL"), &Value::Null)
    );
    assert_eq!(item["created"], json!(created));

    put_known(
        &server,
        &json!({"hash": "unmatched", "action": "DELETE_AUTO"}),
        StatusCode::ACCEPTED,
    )
    .await;
    let missing =
        fetch(server.admin(Method::GET, &format!("{PAGE_HASHES}/unmatched/thumbnail"))).await;
    assert_eq!(
        missing.status,
        StatusCode::NOT_FOUND,
        "no page gave a thumbnail"
    );
    assert_action_filters(&server, &cover_hash).await;
}

async fn assert_action_filters(server: &TestServer, cover_hash: &str) {
    for (query, expected) in [
        ("?sort=hash,desc", vec!["unmatched", cover_hash]),
        ("?action=IGNORE", vec![]),
        (
            "?action=DELETE_AUTO,%20DELETE_MANUAL&sort=matchCount",
            vec!["unmatched", cover_hash],
        ),
        ("?action=DELETE_AUTO&action=&sort=hash", vec!["unmatched"]),
        ("?action=", vec![]),
    ] {
        assert_eq!(hashes(&known(server, query).await), expected, "{query}");
    }
    for (query, value) in [("FOO", "FOO"), ("IGNORE&action=ignore", "ignore")] {
        let response =
            fetch(server.admin(Method::GET, &format!("{PAGE_HASHES}?action={query}"))).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            response.message(),
            format!("Invalid value for parameter 'action': {value}")
        );
    }
}

#[tokio::test]
async fn page_hashes_are_for_admins_after_binding() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let unauthenticated = fetch(server.request(Method::GET, PAGE_HASHES)).await;
    assert_eq!(unauthenticated.status, StatusCode::UNAUTHORIZED);

    let reader = create_reader(&server, "reader@example.com", json!({})).await;
    for path in [
        "",
        "/unknown",
        "/hash",
        "/hash/thumbnail",
        "/unknown/hash/thumbnail",
    ] {
        let response = fetch(reader.get(&server, &format!("{PAGE_HASHES}{path}"))).await;
        assert_eq!(response.status, StatusCode::FORBIDDEN, "{path}");
    }
    let unbound = fetch(reader.get(&server, &format!("{PAGE_HASHES}?action=FOO"))).await;
    assert_eq!(unbound.status, StatusCode::BAD_REQUEST);
    let put = |body: Value| {
        server
            .request(Method::PUT, PAGE_HASHES)
            .basic_auth("reader@example.com", Some("secret"))
            .json(&body)
    };
    let denied = fetch(put(json!({"hash": "hash", "action": "IGNORE"}))).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let match_body = json!({
        "bookId": "B", "url": "/b.cbz", "pageNumber": 1, "fileName": "1.png", "fileSize": 1,
        "mediaType": "image/png",
    });
    for (path, body) in [("delete-all", None), ("delete-match", Some(&match_body))] {
        let mut request = server
            .request(Method::POST, &format!("{PAGE_HASHES}/hash/{path}"))
            .basic_auth("reader@example.com", Some("secret"));
        if let Some(body) = body {
            request = request.json(body);
        }
        assert_eq!(fetch(request).await.status, StatusCode::FORBIDDEN, "{path}");
    }
    let blank = fetch(put(json!({"hash": " ", "action": "IGNORE"}))).await;
    assert_eq!(blank.status, StatusCode::BAD_REQUEST);
    let body: Value = serde_json::from_slice(&blank.body).expect("Violations");
    assert_eq!(
        body["violations"],
        json!([{"fieldName": "hash", "message": "must not be blank"}])
    );

    let invalid = put_known(
        &server,
        &json!({"hash": "hash", "action": "ignore"}),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert!(
        invalid["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("JSON parse error"))
    );
    server.stop().await;
}
