//! `PATCH /api/v1/series/{seriesId}/metadata`.

use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, SqlitePool};

use super::book_edit::violations;
use super::books::pair;
use super::series::fields;
use super::{comic, library, write};
use crate::libraries::admin_json;
use crate::support::TestServer;
use crate::support::db::content::book;
use crate::support::tasks::wait_for_scan_work;

const COMIC_INFO: &str = "<ComicInfo><Series>Found</Series><Publisher>Pub</Publisher>\
    <Genre>Found genre</Genre><Summary>Found summary</Summary></ComicInfo>";

async fn patch(server: &TestServer, path: &str, body: &Value) -> Response {
    server
        .admin(Method::PATCH, path)
        .json(body)
        .send()
        .await
        .expect("Metadata edit")
}

/// The `columns` of a series' rows of `table`, in table order.
async fn rows(pool: &SqlitePool, table: &str, columns: &str, id: &str) -> Vec<(String, String)> {
    let sql = format!("SELECT {columns} FROM {table} WHERE SERIES_ID = ? ORDER BY rowid");
    sqlx::query_as(AssertSqlSafe(sql))
        .bind(id)
        .fetch_all(pool)
        .await
        .expect("Series metadata rows")
}

/// `(STATUS, SUMMARY)`.
async fn status_and_summary(pool: &SqlitePool, id: &str) -> (String, String) {
    sqlx::query_as("SELECT STATUS, SUMMARY FROM SERIES_METADATA WHERE SERIES_ID = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("Series metadata")
}

#[tokio::test]
async fn an_edit_keeps_absent_fields_unsets_null_ones_and_its_locks_survive_refreshes() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    write(
        directory.path(),
        "Series/book.cbz",
        &comic(Some(COMIC_INFO)),
    );
    let (server, _, pool) = library(directory.path()).await;
    let (_, id) = book(&pool, "book").await;
    let route = format!("/api/v1/series/{id}/metadata");
    let summary = json!({"summary": "Mine"});
    assert_eq!(
        patch(&server, &route, &summary).await.status(),
        StatusCode::NO_CONTENT
    );
    let edit = json!({
        "status": "HIATUS",
        "title": " Edited ", "titleLock": true,
        "summary": null,
        "publisher": "",
        "readingDirection": "WEBTOON",
        "ageRating": null,
        "language": "EN-us",
        "genres": null,
        "tags": [" Tag ", "tag"],
        "totalBookCount": 5,
        "sharingLabels": ["Kids"],
        "links": [{"label": "Home", "url": "https://home.example"}],
        "alternateTitles": [{"label": "ja", "title": "タイトル"}],
    });
    assert_eq!(
        patch(&server, &route, &edit).await.status(),
        StatusCode::NO_CONTENT
    );
    let edited = (
        "Edited".to_owned(),
        "Found".to_owned(),
        Some("WEBTOON".to_owned()),
        String::new(),
        None,
        "en-US".to_owned(),
        Some(5),
    );
    assert_eq!(fields(&pool, &id).await, (edited, Vec::new()));
    let (status, summary) = status_and_summary(&pool, &id).await;
    assert_eq!(
        (status.as_str(), summary.as_str()),
        ("HIATUS", "Mine"),
        "null summary kept"
    );
    let tags = rows(&pool, "SERIES_METADATA_TAG", "TAG, SERIES_ID", &id).await;
    assert_eq!(tags, [pair("tag", &id)]);
    let sharing = || rows(&pool, "SERIES_METADATA_SHARING", "LABEL, SERIES_ID", &id);
    assert_eq!(sharing().await, [pair("kids", &id)]);
    let links = || rows(&pool, "SERIES_METADATA_LINK", "LABEL, URL", &id);
    assert_eq!(links().await, [pair("Home", "https://home.example")]);
    let titles = || {
        rows(
            &pool,
            "SERIES_METADATA_ALTERNATE_TITLE",
            "LABEL, TITLE",
            &id,
        )
    };
    assert_eq!(titles().await, [pair("ja", "タイトル")]);

    let refresh = format!("/api/v1/series/{id}/metadata/refresh");
    let response = server.admin(Method::POST, &refresh).send().await;
    assert_eq!(response.expect("Refresh").status(), StatusCode::ACCEPTED);
    wait_for_scan_work(directory.path()).await;
    let ((title, _, _, publisher, _, _, _), genres) = fields(&pool, &id).await;
    assert_eq!(title, "Edited", "locked");
    assert_eq!(
        (publisher.as_str(), genres),
        ("Pub", vec!["found genre".to_owned()])
    );
    assert_eq!(sharing().await, [pair("kids", &id)], "kept by refreshes");
    assert_eq!(links().await, [pair("Home", "https://home.example")]);
    assert_eq!(titles().await, [pair("ja", "タイトル")]);
    server.stop().await;
}

#[tokio::test]
async fn invalid_edits_are_rejected_even_for_readers_and_valid_ones_need_an_admin() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    write(
        directory.path(),
        "Series/book.cbz",
        &comic(Some(COMIC_INFO)),
    );
    let (server, _, pool) = library(directory.path()).await;
    let (_, id) = book(&pool, "book").await;
    let route = format!("/api/v1/series/{id}/metadata");
    let before = fields(&pool, &id).await;
    let invalid = json!({
        "title": "",
        "titleSort": " ",
        "ageRating": -1,
        "language": "klingon",
        "totalBookCount": 0,
        "links": [{"label": " ", "url": "https://home.example"}],
        "alternateTitles": [{"label": "ja"}],
    });
    let expected = [
        ("ageRating", "must be greater than or equal to 0"),
        ("alternateTitles[0].title", "must not be blank"),
        (
            "language",
            "Must be null or blank or valid BCP 47 language tag",
        ),
        ("links[0].label", "must not be blank"),
        ("title", "Must be null or not blank"),
        ("titleSort", "Must be null or not blank"),
        ("totalBookCount", "must be greater than 0"),
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

    let unknown = patch(&server, "/api/v1/series/UNKNOWN/metadata", &json!({})).await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let bad_status = patch(&server, &route, &json!({"status": "SOON"})).await;
    assert_eq!(bad_status.status(), StatusCode::BAD_REQUEST);
    assert_eq!(fields(&pool, &id).await, before, "nothing changed");
    server.stop().await;
}
