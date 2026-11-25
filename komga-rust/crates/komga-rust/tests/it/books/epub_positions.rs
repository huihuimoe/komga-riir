//! Positions of analyzed EPUB books (`BookController.getBookPositions`).

use reqwest::header::{CONTENT_TYPE, IF_MODIFIED_SINCE, LAST_MODIFIED};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::epub::sample;
use super::reading::access::create_reader;
use super::reading::fetch;
use super::{PNG, library_with_books};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;

#[tokio::test]
async fn epub_positions_are_the_analyzed_locators() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let png = cbz::fixture(PNG);
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            (
                "Series/theft.epub",
                sample("The Incomplete Theft - Ralph Burke.epub"),
            ),
            ("Series/comic.cbz", cbz::zip(&[("p.png", STORED, &png)])),
        ],
    )
    .await;
    let theft = book(&pool, "theft").await.0;
    let path = format!("/api/v1/books/{theft}/positions");

    let response = fetch(server.admin(Method::GET, &path)).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.header(&CONTENT_TYPE),
        "application/vnd.readium.position-list+json"
    );
    let body: Value = serde_json::from_slice(&response.body).expect("JSON positions");
    assert_eq!(body["total"], json!(35));
    assert_eq!(body["positions"].as_array().map(Vec::len), Some(35));
    // The last of 31 positions in the last document: progression 30/31, total 35/35.
    assert_eq!(
        body["positions"][34],
        json!({
            "href": "OEBPS/@public@vhost@g@gutenberg@html@files@65659@65659-h@65659-h-0.htm_split_002.html",
            "type": "application/xhtml+xml",
            "locations": {"progression": 0.967_741_9, "position": 35, "totalProgression": 1.0},
            "koboSpan": "kobo.50.70",
        })
    );

    let conditional = server
        .admin(Method::GET, &path)
        .header(IF_MODIFIED_SINCE, response.header(&LAST_MODIFIED));
    assert_eq!(fetch(conditional).await.status, StatusCode::NOT_MODIFIED);

    let comic = book(&pool, "comic").await.0;
    let no_extension =
        fetch(server.admin(Method::GET, &format!("/api/v1/books/{comic}/positions")));
    assert_eq!(no_extension.await.status, StatusCode::NOT_FOUND);
    let unknown = fetch(server.admin(Method::GET, "/api/v1/books/0/positions"));
    assert_eq!(unknown.await.status, StatusCode::NOT_FOUND);

    let anonymous = fetch(server.request(Method::GET, &path)).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    let outsider = create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    let denied = fetch(outsider.get(&server, &path)).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.message(), "403 FORBIDDEN");
    server.stop().await;
}
