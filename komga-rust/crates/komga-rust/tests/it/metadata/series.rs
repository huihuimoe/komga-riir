//! `RefreshSeriesMetadata` aggregating the `ComicInfo.xml` of the books of a series.

use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use super::{comic, library, rescan_deep, write};
use crate::libraries::{admin_json, route};
use crate::support::db::content::book;

/// `(TITLE, TITLE_SORT, READING_DIRECTION, PUBLISHER, AGE_RATING, LANGUAGE, TOTAL_BOOK_COUNT)`.
pub(super) type Fields = (
    String,
    String,
    Option<String>,
    String,
    Option<i32>,
    String,
    Option<i32>,
);

pub(super) async fn fields(pool: &SqlitePool, series_id: &str) -> (Fields, Vec<String>) {
    let fields = sqlx::query_as(
        "SELECT TITLE, TITLE_SORT, READING_DIRECTION, PUBLISHER, AGE_RATING, LANGUAGE, \
         TOTAL_BOOK_COUNT FROM SERIES_METADATA WHERE SERIES_ID = ?",
    )
    .bind(series_id)
    .fetch_one(pool)
    .await
    .expect("Series metadata");
    let genres = sqlx::query_scalar(
        "SELECT GENRE FROM SERIES_METADATA_GENRE WHERE SERIES_ID = ? ORDER BY 1",
    )
    .bind(series_id)
    .fetch_all(pool)
    .await
    .expect("Series genres");
    (fields, genres)
}

const FIRST: &str = "<ComicInfo><Series>Saga</Series><Volume>2</Volume><Count>3</Count>\
    <Manga>YesAndRightToLeft</Manga><Publisher>Image</Publisher><AgeRating>Teen</AgeRating>\
    <LanguageISO>EN</LanguageISO><Genre>Space</Genre></ComicInfo>";
const SECOND: &str = "<ComicInfo><Series>Saga</Series><Volume>2</Volume><Count>5</Count>\
    <AgeRating>Mature 17+</AgeRating><Genre>Opera, space</Genre></ComicInfo>";
const THIRD: &str = "<ComicInfo><Series>Other</Series><LanguageISO>klingon</LanguageISO>\
    </ComicInfo>";

#[tokio::test]
async fn a_series_takes_the_most_frequent_comic_info_of_its_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    for (name, xml) in [("a", FIRST), ("b", SECOND), ("c", THIRD)] {
        write(
            directory.path(),
            &format!("Folder/{name}.cbz"),
            &comic(Some(xml)),
        );
    }
    let (server, library, pool) = library(directory.path()).await;
    let (_, series_id) = book(&pool, "a").await;
    let expected = |title: &str| {
        (
            (
                title.to_owned(),
                title.to_owned(),
                Some("RIGHT_TO_LEFT".to_owned()),
                "Image".to_owned(),
                Some(17),
                "en".to_owned(),
                Some(5),
            ),
            vec!["opera".to_owned(), "space".to_owned()],
        )
    };
    assert_eq!(fields(&pool, &series_id).await, expected("Saga (2)"));

    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"importComicInfoSeriesAppendVolume": false})),
        StatusCode::NO_CONTENT,
    )
    .await;
    rescan_deep(&server, directory.path(), &library).await;
    assert_eq!(fields(&pool, &series_id).await, expected("Saga"));

    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"importComicInfoSeries": false, "importComicInfoSeriesAppendVolume": true})),
        StatusCode::NO_CONTENT,
    )
    .await;
    rescan_deep(&server, directory.path(), &library).await;
    assert_eq!(
        fields(&pool, &series_id).await,
        expected("Saga"),
        "the library imports no ComicInfo series metadata"
    );
    server.stop().await;
}
