//! Series metadata from the Mylar `series.json` of a series directory (`MylarSeriesProvider`).

use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use super::{comic, library, rescan_deep, write};
use crate::libraries::{admin_json, route};
use crate::support::db::content::book;

/// `(TITLE, TITLE_SORT, STATUS, SUMMARY, PUBLISHER, AGE_RATING, TOTAL_BOOK_COUNT)`.
type Fields = (
    String,
    String,
    String,
    String,
    String,
    Option<i32>,
    Option<i32>,
);

async fn fields(pool: &SqlitePool, series_id: &str) -> Fields {
    sqlx::query_as(
        "SELECT TITLE, TITLE_SORT, STATUS, SUMMARY, PUBLISHER, AGE_RATING, TOTAL_BOOK_COUNT \
         FROM SERIES_METADATA WHERE SERIES_ID = ?",
    )
    .bind(series_id)
    .fetch_one(pool)
    .await
    .expect("Series metadata")
}

fn series_json(directory: &Path, name: &str, volume: i32) {
    let document = json!({
        "version": "1.0.1",
        "metadata": {
            "type": "comicSeries",
            "publisher": "IDW Publishing",
            "imprint": null,
            "name": name,
            "comicid": 119_731,
            "year": 2019,
            "description_text": "Text",
            "description_formatted": null,
            "volume": volume,
            "booktype": "Print",
            "age_rating": "12+",
            "comic_image": "https://example.com/cover.jpg",
            "total_issues": 20,
            "publication_run": "June 2019 - Present",
            "status": "Ended"
        }
    });
    let content = serde_json::to_vec(&document).expect("JSON");
    write(directory, "Folder/series.json", &content);
}

fn expected(title: &str) -> Fields {
    (
        title.to_owned(),
        title.to_owned(),
        "ENDED".to_owned(),
        "Text".to_owned(),
        "IDW Publishing".to_owned(),
        Some(12),
        Some(20),
    )
}

#[tokio::test]
async fn a_series_takes_its_series_json_unless_unreadable_or_not_imported() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    write(directory.path(), "Folder/a.cbz", &comic(None));
    series_json(directory.path(), "Usagi Yojimbo", 4);
    let (server, library, pool) = library(directory.path()).await;
    let (_, series_id) = book(&pool, "a").await;
    assert_eq!(
        fields(&pool, &series_id).await,
        expected("Usagi Yojimbo (2019)")
    );

    write(
        directory.path(),
        "Folder/series.json",
        b"{\"metadata\": {}}",
    );
    rescan_deep(&server, directory.path(), &library).await;
    assert_eq!(
        fields(&pool, &series_id).await,
        expected("Usagi Yojimbo (2019)"),
        "an invalid series.json changes nothing"
    );

    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"importMylarSeries": false})),
        StatusCode::NO_CONTENT,
    )
    .await;
    series_json(directory.path(), "Other", 1);
    rescan_deep(&server, directory.path(), &library).await;
    assert_eq!(
        fields(&pool, &series_id).await,
        expected("Usagi Yojimbo (2019)"),
        "the library imports no Mylar metadata"
    );
    server.stop().await;
}
