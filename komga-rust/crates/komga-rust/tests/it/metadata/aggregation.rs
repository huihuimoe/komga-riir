//! `AggregateSeriesMetadata`: the book metadata of a series aggregated after each series refresh
//! and book edit.

use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use super::books::pair;
use super::{comic, library, rescan_deep, write};
use crate::support::db::content::{self, book};
use crate::support::tasks::wait_for_scan_work;

/// `(RELEASE_DATE, SUMMARY, SUMMARY_NUMBER)`, then the sorted authors, notable entities, and
/// tags; table order is not part of the contract.
type Aggregation = (
    (Option<String>, String, String),
    Vec<(String, String)>,
    Vec<(String, String)>,
    Vec<String>,
);

async fn aggregation(pool: &SqlitePool, series_id: &str) -> Aggregation {
    let fields = sqlx::query_as(
        "SELECT RELEASE_DATE, SUMMARY, SUMMARY_NUMBER FROM BOOK_METADATA_AGGREGATION \
         WHERE SERIES_ID = ?",
    )
    .bind(series_id)
    .fetch_one(pool)
    .await
    .expect("Aggregation");
    let authors = rows(
        pool,
        "SELECT NAME, ROLE FROM BOOK_METADATA_AGGREGATION_AUTHOR WHERE SERIES_ID = ? ORDER BY 1, 2",
        series_id,
    )
    .await;
    let entities = rows(
        pool,
        "SELECT KIND, NAME FROM BOOK_METADATA_AGGREGATION_NOTABLE_ENTITIES WHERE SERIES_ID = ? \
         ORDER BY 1, 2",
        series_id,
    )
    .await;
    let tags = sqlx::query_scalar(
        "SELECT TAG FROM BOOK_METADATA_AGGREGATION_TAG WHERE SERIES_ID = ? ORDER BY 1",
    )
    .bind(series_id)
    .fetch_all(pool)
    .await
    .expect("Aggregation tags");
    (fields, authors, entities, tags)
}

async fn rows(pool: &SqlitePool, sql: &'static str, series_id: &str) -> Vec<(String, String)> {
    sqlx::query_as(sql)
        .bind(series_id)
        .fetch_all(pool)
        .await
        .expect("Aggregation rows")
}

const SECOND: &str = "<ComicInfo><Number>2</Number><Summary>Second</Summary><Year>2021</Year>\
    <Writer>Ann</Writer><Penciller>Bob</Penciller><Characters>Eve</Characters>\
    <Tags>b, a</Tags></ComicInfo>";
const FIRST: &str = "<ComicInfo><Number>1</Number><Year>2019</Year><Month>3</Month>\
    <Writer>Ann</Writer><Characters>Eve</Characters><Locations>Eve</Locations>\
    <Tags>a, c</Tags></ComicInfo>";

#[tokio::test]
async fn a_series_aggregates_its_books_again_on_each_refresh_and_edit() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    for (name, xml) in [("a", SECOND), ("b", FIRST)] {
        write(
            directory.path(),
            &format!("Folder/{name}.cbz"),
            &comic(Some(xml)),
        );
    }
    let (server, library, pool) = library(directory.path()).await;
    let (first_id, series_id) = book(&pool, "b").await;
    let expected = |summary: &str, number: &str| -> Aggregation {
        (
            (
                Some("2019-03-01".to_owned()),
                summary.to_owned(),
                number.to_owned(),
            ),
            vec![pair("Ann", "writer"), pair("Bob", "penciller")],
            vec![pair("character", "Eve"), pair("location", "Eve")],
            vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
        )
    };
    // The first book by number has no summary, so the second one's is taken.
    assert_eq!(
        aggregation(&pool, &series_id).await,
        expected("Second", "2")
    );

    // Repeated refreshes aggregate again without duplicating anything.
    for _ in 0..2 {
        let before = content::aggregation_dates(&pool).await;
        rescan_deep(&server, directory.path(), &library).await;
        assert_eq!(
            content::aggregated_since(&pool, &before).await,
            [series_id.as_str()]
        );
        assert_eq!(
            aggregation(&pool, &series_id).await,
            expected("Second", "2")
        );
    }

    let response = server
        .admin(Method::PATCH, &format!("/api/v1/books/{first_id}/metadata"))
        .json(&json!({"summary": "First"}))
        .send()
        .await
        .expect("Metadata edit");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    wait_for_scan_work(directory.path()).await;
    assert_eq!(aggregation(&pool, &series_id).await, expected("First", "1"));
    server.stop().await;
}
