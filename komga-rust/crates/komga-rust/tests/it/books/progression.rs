//! Readium progressions (`CommonBookController.getBookProgression`/`updateBookProgression`).

use std::path::Path;

use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::epub::sample;
use super::library_with_books;
use super::read_progress::{mark, progress};
use super::reading::access::create_reader;
use super::reading::{Fetched, fetch, pages_book};
use crate::support::db::content::{book, media};
use crate::support::{ADMIN_EMAIL, TestServer, openapi};

const PROGRESSION_JSON: &str = "application/vnd.readium.progression+json";

fn route(book_id: &str) -> String {
    format!("/api/v1/books/{book_id}/progression")
}

fn progression(modified: &str, locator: Value) -> Value {
    json!({"modified": modified, "device": {"id": "d1", "name": "Reader"}, "locator": locator})
}

async fn put(server: &TestServer, book_id: &str, body: &Value) -> Fetched {
    fetch(server.admin(Method::PUT, &route(book_id)).json(body)).await
}

#[tokio::test]
async fn image_progressions_are_page_positions_per_user() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/one.cbz", pages_book())]).await;
    let one = book(&pool, "one").await.0;

    // `ResponseEntity.noContent()`, which springdoc does not document.
    let empty = fetch(server.admin(Method::GET, &route(&one))).await;
    assert_eq!(empty.status, StatusCode::NO_CONTENT);

    let locator = json!({"href": "p2.jpg", "type": "image/jpeg", "locations": {"position": 2}});
    let body = progression("2024-01-02T03:04:05.5+09:00", locator.clone());
    let response = server
        .admin(Method::PUT, &route(&one))
        .json(&body)
        .send()
        .await;
    let response = response.expect("Put");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    openapi::checked(Method::PUT, response).await;
    let stored = (
        ADMIN_EMAIL.to_owned(),
        2,
        false,
        "d1".to_owned(),
        "Reader".to_owned(),
    );
    assert_eq!(progress(&pool, &one).await, [stored]);
    // Not checked against OpenAPI: springdoc marks `fragments` required, but `NON_EMPTY` omits
    // an empty list.
    for path in [route(&one), format!("/opds/v2/books/{one}/progression")] {
        let response = fetch(server.admin(Method::GET, &path)).await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.header(&CONTENT_TYPE), PROGRESSION_JSON);
        assert_eq!(
            serde_json::from_slice::<Value>(&response.body).expect("JSON progression"),
            json!({
                "modified": "2024-01-01T18:04:05.5Z",
                "device": {"id": "d1", "name": "Reader"},
                "locator": locator,
            }),
            "{path}"
        );
    }

    // Only a later progression replaces the stored one.
    for modified in ["2024-01-01T18:04:05.5Z", "2024-01-01T18:00:00Z"] {
        let stale = put(&server, &one, &progression(modified, locator.clone())).await;
        assert_eq!(stale.status, StatusCode::CONFLICT, "{modified}");
        assert_eq!(stale.message(), "Progression is older than existing");
    }
    let last = json!({"href": "p10.png", "type": "image/png", "locations": {"position": 3}});
    let later = put(&server, &one, &progression("2024-01-03T00:00:00Z", last)).await;
    assert_eq!(later.status, StatusCode::NO_CONTENT);
    let completed = (
        ADMIN_EMAIL.to_owned(),
        3,
        true,
        "d1".to_owned(),
        "Reader".to_owned(),
    );
    assert_eq!(progress(&pool, &one).await, [completed]);

    // A position is required within the book; an EPUB locator is not converted to a page.
    for (locations, page) in [
        (json!({"position": 4}), "4"),
        (json!({"progression": 0.5}), "null"),
    ] {
        let locator = json!({"href": "p2.jpg", "type": "image/jpeg", "locations": locations});
        let rejected = put(&server, &one, &progression("2024-01-04T00:00:00Z", locator)).await;
        assert_eq!(rejected.status, StatusCode::BAD_REQUEST, "{locations}");
        assert_eq!(
            rejected.message(),
            format!("Page argument ({page}) must be within 1 and book page count (3)")
        );
    }

    // Another user's progression is their own.
    let reader = create_reader(&server, "reader@example.com", json!({"roles": []})).await;
    let theirs = fetch(reader.get(&server, &route(&one))).await;
    assert_eq!(theirs.status, StatusCode::NO_CONTENT);
    server.stop().await;
}

#[tokio::test]
async fn pages_marked_through_read_progress_have_an_empty_locator_and_no_device() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/one.cbz", pages_book())]).await;
    let one = book(&pool, "one").await.0;
    mark(&server, &one, json!({"page": 1})).await;

    let stored = fetch(server.admin(Method::GET, &route(&one))).await;
    let stored: Value = serde_json::from_slice(&stored.body).expect("JSON progression");
    assert_eq!(stored["device"], json!({"id": "", "name": ""}));
    assert_eq!(stored["locator"], json!({}));
    assert!(
        stored["modified"]
            .as_str()
            .is_some_and(|modified| modified.ends_with('Z')),
        "{stored}"
    );
    server.stop().await;
}

#[tokio::test]
async fn invalid_progressions_unknown_and_hidden_books_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/one.cbz", pages_book())]).await;
    let one = book(&pool, "one").await.0;
    let valid = progression(
        "2024-01-02T03:04:05Z",
        json!({"href": "p2.jpg", "type": "image/jpeg", "locations": {"position": 1}}),
    );

    for body in [
        json!({"modified": "2024-01-02T03:04:05Z", "locator": valid["locator"]}),
        progression("2024-01-02T03:04:05", valid["locator"].clone()),
        progression("2024-01-02T03:04:05Z", json!({"type": "image/jpeg"})),
    ] {
        assert_eq!(
            put(&server, &one, &body).await.status,
            StatusCode::BAD_REQUEST,
            "{body}"
        );
    }
    assert_eq!(
        put(&server, "0", &valid).await.status,
        StatusCode::NOT_FOUND
    );
    let unknown = fetch(server.admin(Method::GET, &route("0")));
    assert_eq!(unknown.await.status, StatusCode::NOT_FOUND);

    let outsider = create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    for method in [Method::GET, Method::PUT] {
        let denied = fetch(
            outsider
                .request(&server, method.clone(), &route(&one))
                .json(&valid),
        );
        let denied = denied.await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN, "{method}");
        assert_eq!(denied.message(), "403 FORBIDDEN");
    }
    let anonymous = fetch(server.request(Method::GET, &route(&one))).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert!(progress(&pool, &one).await.is_empty());
    server.stop().await;
}

/// The resource of an analyzed position and the positions it holds.
fn positions_of<'a>(positions: &'a [Value], href: &str) -> Vec<&'a Value> {
    positions
        .iter()
        .filter(|position| position["href"] == href)
        .collect()
}

/// A server whose library holds an analyzed reflowable EPUB, with its ID and positions.
async fn theft_library(directory: &Path) -> (TestServer, SqlitePool, String, Vec<Value>) {
    let (server, _, pool) = library_with_books(
        directory,
        &[(
            "Series/theft.epub",
            sample("The Incomplete Theft - Ralph Burke.epub"),
        )],
    )
    .await;
    let theft = book(&pool, "theft").await.0;
    let positions = fetch(server.admin(Method::GET, &format!("/api/v1/books/{theft}/positions")));
    let positions: Value = serde_json::from_slice(&positions.await.body).expect("JSON positions");
    let positions = positions["positions"]
        .as_array()
        .expect("Positions")
        .clone();
    (server, pool, theft, positions)
}

#[tokio::test]
async fn epub_progressions_take_the_analyzed_total_progression() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool, theft, positions) = theft_library(directory.path()).await;
    let page_count = media(&pool, &theft).await.3;
    let last = &positions[34];
    let href = last["href"].as_str().expect("Href");

    // An exact match: the analyzed type, Kobo span, and total progression replace the reported.
    let locator = json!({
        "href": format!("{href}#part"),
        "type": "text/html",
        "locations": {"progression": last["locations"]["progression"], "totalProgression": 0.1},
    });
    let response = put(
        &server,
        &theft,
        &progression("2024-01-02T00:00:00Z", locator),
    )
    .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    let stored = fetch(server.admin(Method::GET, &route(&theft))).await;
    let stored: Value = serde_json::from_slice(&stored.body).expect("JSON progression");
    assert_eq!(
        stored["locator"],
        json!({
            "href": format!("{href}#part"),
            "type": "application/xhtml+xml",
            "locations": {"progression": last["locations"]["progression"], "totalProgression": 1.0},
            "koboSpan": "kobo.50.70",
        })
    );
    let stored = progress(&pool, &theft).await;
    assert_eq!((stored[0].1, stored[0].2), (page_count, true));

    // Between two positions of a resource, the earlier one counts.
    let resource = positions
        .iter()
        .map(|position| positions_of(&positions, position["href"].as_str().expect("Href")))
        .find(|resource| resource.len() > 2)
        .expect("A resource with several positions");
    let (first, second) = (&resource[0]["locations"], &resource[1]["locations"]);
    let between = (first["progression"].as_f64().expect("Progression")
        + second["progression"].as_f64().expect("Progression"))
        / 2.0;
    let locator = json!({
        "href": resource[0]["href"],
        "type": "application/xhtml+xml",
        "locations": {"progression": between},
        "koboSpan": "mine",
    });
    let response = put(
        &server,
        &theft,
        &progression("2024-01-03T00:00:00Z", locator),
    )
    .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    let total = first["totalProgression"]
        .as_f64()
        .expect("Total progression");
    #[expect(clippy::cast_possible_truncation, reason = "a page number")]
    let page = (f64::from(page_count) * total).round() as i32;
    assert_eq!(progress(&pool, &theft).await[0].1, page);
    let stored = fetch(server.admin(Method::GET, &route(&theft))).await;
    let stored: Value = serde_json::from_slice(&stored.body).expect("JSON progression");
    assert_eq!(
        stored["locator"]["koboSpan"], "mine",
        "a reported Kobo span is kept"
    );

    server.stop().await;
}

#[tokio::test]
async fn epub_progressions_need_a_known_resource_and_a_matching_position() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, theft, positions) = theft_library(directory.path()).await;
    let last = &positions[34];
    let href = last["href"].as_str().expect("Href");
    let after_last = last["locations"]["progression"]
        .as_f64()
        .expect("Progression")
        + 0.01;
    for (locator, message) in [
        (
            json!({"href": href, "type": "", "locations": {"progression": after_last}}),
            "Invalid progression".to_owned(),
        ),
        (
            json!({"href": href, "type": ""}),
            "location.progression is required".to_owned(),
        ),
        (
            json!({"href": "nope.html", "type": "", "locations": {"progression": 0.5}}),
            "Resource does not exist in book: nope.html".to_owned(),
        ),
        (
            json!({"href": "100%", "type": "", "locations": {"progression": 0.5}}),
            "Malformed escape in href \"100%\"".to_owned(),
        ),
    ] {
        let rejected = put(
            &server,
            &theft,
            &progression("2024-01-04T00:00:00Z", locator),
        )
        .await;
        assert_eq!(rejected.status, StatusCode::BAD_REQUEST, "{message}");
        assert_eq!(rejected.message(), message);
    }

    // The image page route does not turn a page into an EPUB locator.
    let page = mark(&server, &theft, json!({"page": 1})).await;
    assert_eq!(page.status(), StatusCode::BAD_REQUEST);
    server.stop().await;
}
