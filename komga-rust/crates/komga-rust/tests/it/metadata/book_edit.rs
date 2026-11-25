//! `PATCH /api/v1/books/{bookId}/metadata` and `PATCH /api/v1/books/metadata`.

use std::path::Path;

use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::books::{fields, pair, pairs};
use super::{comic, library, write};
use crate::libraries::admin_json;
use crate::support::db::content::{self, book};
use crate::support::tasks::{clear_after_scans, wait_for_scan_work};
use crate::support::{TestServer, json};

const COMIC_INFO: &str = r#"<ComicInfo>
  <Title>Found</Title><Summary>Found summary</Summary><Number>3</Number>
  <Year>2020</Year><Writer>Wanda</Writer><Characters>Eve</Characters><Tags>found</Tags>
  <Web>https://found.example</Web>
</ComicInfo>"#;

async fn patch(server: &TestServer, path: &str, body: &Value) -> Response {
    server
        .admin(Method::PATCH, path)
        .json(body)
        .send()
        .await
        .expect("Metadata edit")
}

/// The `(fieldName, message)` pairs of a validation error response, sorted.
pub(super) async fn violations(response: Response) -> Vec<(String, String)> {
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json(response).await;
    let mut violations: Vec<(String, String)> = body["violations"]
        .as_array()
        .expect("Validation errors list violations")
        .iter()
        .map(|violation| {
            let text = |key: &str| violation[key].as_str().expect("Violation text").to_owned();
            (text("fieldName"), text("message"))
        })
        .collect();
    violations.sort();
    violations
}

async fn one_book(directory: &Path) -> (TestServer, SqlitePool, String) {
    write(directory, "Series/book.cbz", &comic(Some(COMIC_INFO)));
    let (server, _, pool) = library(directory).await;
    let (id, _) = book(&pool, "book").await;
    (server, pool, id)
}

#[tokio::test]
async fn an_edit_keeps_absent_fields_unsets_null_ones_and_its_locks_survive_refreshes() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool, id) = one_book(directory.path()).await;
    let route = format!("/api/v1/books/{id}/metadata");
    let edit = json!({
        "title": " Edited ", "titleLock": true,
        "summary": null,
        "number": null,
        "releaseDate": null,
        "authors": [{"name": " Ann ", "role": "Penciller"}], "authorsLock": true,
        "tags": null,
        "isbn": "ISBN 978-2-8116-3239-7",
        "links": [{"label": "Home", "url": "https://home.example/a"}],
    });
    clear_after_scans(&server, directory.path()).await;
    let aggregations = content::aggregation_dates(&pool).await;
    assert_eq!(
        patch(&server, &route, &edit).await.status(),
        StatusCode::NO_CONTENT
    );

    let edited = (
        "Edited".to_owned(),
        String::new(),
        "3".to_owned(),
        3.0,
        None,
        "9782811632397".to_owned(),
    );
    assert_eq!(fields(&pool, &id).await, edited);
    let authors = || pairs(&pool, "BOOK_METADATA_AUTHOR", "NAME, ROLE", &id);
    assert_eq!(authors().await, [pair("Ann", "penciller")]);
    let entities = || pairs(&pool, "BOOK_METADATA_NOTABLE_ENTITIES", "KIND, NAME", &id);
    assert_eq!(entities().await, [pair("character", "Eve")], "absent");
    let tags = pairs(&pool, "BOOK_METADATA_TAG", "TAG, BOOK_ID", &id).await;
    assert_eq!(tags, []);
    let links = pairs(&pool, "BOOK_METADATA_LINK", "LABEL, URL", &id).await;
    assert_eq!(links, [pair("Home", "https://home.example/a")]);
    wait_for_scan_work(directory.path()).await;
    let (_, series_id) = content::book(&pool, "book").await;
    assert_eq!(
        content::aggregated_since(&pool, &aggregations).await,
        [series_id],
        "the edit aggregates the series again"
    );

    let refresh = format!("/api/v1/books/{id}/metadata/refresh");
    let response = server.admin(Method::POST, &refresh).send().await;
    assert_eq!(response.expect("Refresh").status(), StatusCode::ACCEPTED);
    wait_for_scan_work(directory.path()).await;
    let (title, summary, _, _, release_date, _) = fields(&pool, &id).await;
    assert_eq!(title, "Edited", "locked");
    assert_eq!(summary, "Found summary", "unlocked");
    assert_eq!(release_date.as_deref(), Some("2020-01-01"));
    assert_eq!(authors().await, [pair("Ann", "penciller")], "locked");
    server.stop().await;
}

#[tokio::test]
async fn invalid_edits_are_rejected_even_for_readers_and_valid_ones_need_an_admin() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool, id) = one_book(directory.path()).await;
    let route = format!("/api/v1/books/{id}/metadata");
    let before = fields(&pool, &id).await;
    let invalid = json!({
        "title": " ",
        "number": "",
        "authors": [{"name": "Ann"}],
        "notableEntities": [{"kind": " ", "name": "Eve"}],
        "isbn": "9782811632398",
        "links": [{"label": "Home", "url": "not a url"}, {"label": "Blank", "url": ""}],
    });
    let expected = [
        ("authors[0].role", "must not be blank"),
        ("isbn", "Must be null or blank or valid ISBN-13"),
        ("links[0].url", "must be a valid URL"),
        ("notableEntities[0].kind", "must not be blank"),
        ("number", "Must be null or not blank"),
        ("title", "Must be null or not blank"),
    ]
    .map(|(field, message)| (field.to_owned(), message.to_owned()));
    assert_eq!(
        violations(patch(&server, &route, &invalid).await).await,
        expected
    );

    let reader = json!({"email": "reader@example.com", "password": "readerpass"});
    admin_json(
        &server,
        Method::POST,
        "/api/v2/users",
        Some(&reader),
        StatusCode::CREATED,
    )
    .await;
    let as_reader = |body: &Value| {
        server
            .request(Method::PATCH, &route)
            .basic_auth("reader@example.com", Some("readerpass"))
            .json(body)
            .send()
    };
    let response = as_reader(&invalid).await.expect("Invalid reader edit");
    assert_eq!(violations(response).await, expected);
    let response = as_reader(&json!({"title": "Mine"}))
        .await
        .expect("Reader edit");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let unknown = patch(&server, "/api/v1/books/UNKNOWN/metadata", &json!({})).await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let bad_date = patch(&server, &route, &json!({"releaseDate": "soon"})).await;
    assert_eq!(bad_date.status(), StatusCode::BAD_REQUEST);
    assert_eq!(fields(&pool, &id).await, before, "nothing changed");
    server.stop().await;
}

#[tokio::test]
async fn a_batch_edit_skips_unknown_books_and_an_invalid_one_changes_nothing() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    write(directory.path(), "Series/a.cbz", &comic(None));
    write(directory.path(), "Series/b.cbz", &comic(None));
    let (server, _, pool) = library(directory.path()).await;
    let (a, _) = book(&pool, "a").await;
    let (b, _) = book(&pool, "b").await;
    let route = "/api/v1/books/metadata";

    let invalid = json!({ a.clone(): {"title": "A"}, b.clone(): {"title": ""} });
    let response = patch(&server, route, &invalid).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json(response).await;
    assert_eq!(body["message"], "Validation failure");
    assert!(body.get("violations").is_none());
    assert_eq!(fields(&pool, &a).await.0, "a", "nothing changed");

    let edit = json!({
        a.clone(): {"title": "A"},
        "UNKNOWN": {"title": "X"},
        b.clone(): {"numberSort": 9.5},
    });
    assert_eq!(
        patch(&server, route, &edit).await.status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(fields(&pool, &a).await.0, "A");
    let (title, _, _, number_sort, _, _) = fields(&pool, &b).await;
    assert_eq!((title.as_str(), number_sort), ("b", 9.5));
    server.stop().await;
}
