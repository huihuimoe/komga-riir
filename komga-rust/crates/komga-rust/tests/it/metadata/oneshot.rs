//! Series metadata of a oneshot from its book (`OneShotSeriesProvider`).

use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use super::{comic, library, rescan_deep, write};
use crate::libraries::{admin_json, route};
use crate::support::db::content::book;

/// `(TITLE, TITLE_SORT, STATUS, SUMMARY, TOTAL_BOOK_COUNT)`.
type Fields = (String, String, String, String, Option<i32>);

async fn fields(pool: &SqlitePool, series_id: &str) -> Fields {
    sqlx::query_as(
        "SELECT TITLE, TITLE_SORT, STATUS, SUMMARY, TOTAL_BOOK_COUNT \
         FROM SERIES_METADATA WHERE SERIES_ID = ?",
    )
    .bind(series_id)
    .fetch_one(pool)
    .await
    .expect("Series metadata")
}

const COMIC_INFO: &str = "<ComicInfo><Title>One Title</Title><Summary>Once</Summary>\
    <Series>Ignored</Series><Count>5</Count></ComicInfo>";

#[tokio::test]
async fn a_oneshot_series_takes_the_title_and_summary_of_its_book() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    write(
        directory.path(),
        "_oneshots/one.cbz",
        &comic(Some(COMIC_INFO)),
    );
    // A series directory's Mylar metadata; the oneshot series is its book file.
    write(directory.path(), "_oneshots/series.json", b"{}");
    let (server, library, pool) = library(directory.path()).await;
    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"oneshotsDirectory": "_oneshots"})),
        StatusCode::NO_CONTENT,
    )
    .await;
    rescan_deep(&server, directory.path(), &library).await;
    let (_, series_id) = book(&pool, "one").await;
    assert_eq!(
        fields(&pool, &series_id).await,
        (
            "One Title".to_owned(),
            "One Title".to_owned(),
            "ENDED".to_owned(),
            "Once".to_owned(),
            Some(1),
        ),
        "the oneshot provider, applied after ComicInfo, sets the total count to 1"
    );
    server.stop().await;
}
