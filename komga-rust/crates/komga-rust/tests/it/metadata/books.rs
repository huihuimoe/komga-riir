//! `RefreshBookMetadata` after analyses and sorts, from `ComicInfo.xml`.

use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::{AssertSqlSafe, SqlitePool};

use super::{comic, library, rescan, write};
use crate::libraries::{admin_json, route};
use crate::support::db::content::book;

/// `(TITLE, SUMMARY, NUMBER, NUMBER_SORT, RELEASE_DATE, ISBN)`.
pub(super) type Fields = (String, String, String, f64, Option<String>, String);

pub(super) async fn fields(pool: &SqlitePool, book_id: &str) -> Fields {
    sqlx::query_as(
        "SELECT TITLE, SUMMARY, NUMBER, NUMBER_SORT, RELEASE_DATE, ISBN \
         FROM BOOK_METADATA WHERE BOOK_ID = ?",
    )
    .bind(book_id)
    .fetch_one(pool)
    .await
    .expect("Book metadata")
}

/// The `(first, second)` columns of a metadata child table, in table order.
pub(super) async fn pairs(
    pool: &SqlitePool,
    table: &str,
    columns: &str,
    book_id: &str,
) -> Vec<(String, String)> {
    let sql = format!("SELECT {columns} FROM {table} WHERE BOOK_ID = ? ORDER BY rowid");
    sqlx::query_as(AssertSqlSafe(sql))
        .bind(book_id)
        .fetch_all(pool)
        .await
        .expect("Metadata rows")
}

pub(super) fn pair(first: &str, second: &str) -> (String, String) {
    (first.to_owned(), second.to_owned())
}

const FULL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<ComicInfo xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <Title> The Title </Title>
  <Summary>A &amp; B</Summary>
  <Number>2.5</Number>
  <Year>2021</Year><Month>3</Month>
  <Writer>Alice, Bob</Writer><CoverArtist>Carol</CoverArtist>
  <Characters>Eve</Characters><Teams>Team</Teams>
  <Web>https://example.com/book not-a-link</Web>
  <Tags>Fun, SAD</Tags>
  <GTIN>2-8116-3239-5</GTIN>
</ComicInfo>"#;

#[tokio::test]
async fn an_analyzed_book_takes_the_metadata_of_its_comic_info() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    write(directory.path(), "Series/book.cbz", &comic(Some(FULL)));
    let (server, _, pool) = library(directory.path()).await;
    let (id, _) = book(&pool, "book").await;

    assert_eq!(
        fields(&pool, &id).await,
        (
            "The Title".to_owned(),
            "A & B".to_owned(),
            "2.5".to_owned(),
            2.5,
            Some("2021-03-01".to_owned()),
            "9782811632397".to_owned(),
        )
    );
    assert_eq!(
        pairs(&pool, "BOOK_METADATA_AUTHOR", "NAME, ROLE", &id).await,
        [
            pair("Alice", "writer"),
            pair("Bob", "writer"),
            pair("Carol", "cover")
        ]
    );
    assert_eq!(
        pairs(&pool, "BOOK_METADATA_NOTABLE_ENTITIES", "KIND, NAME", &id).await,
        [pair("team", "Team"), pair("character", "Eve")]
    );
    assert_eq!(
        pairs(&pool, "BOOK_METADATA_LINK", "LABEL, URL", &id).await,
        [pair("example.com", "https://example.com/book")]
    );
    assert_eq!(
        pairs(&pool, "BOOK_METADATA_TAG", "TAG, BOOK_ID", &id).await,
        [pair("fun", &id), pair("sad", &id)]
    );
    server.stop().await;
}

#[tokio::test]
async fn an_unreadable_comic_info_or_a_disabled_import_keeps_the_stored_metadata() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let title = |title: &str| format!("<ComicInfo><Title>{title}</Title></ComicInfo>");
    write(
        directory.path(),
        "Series/book.cbz",
        &comic(Some(&title("First"))),
    );
    let (server, library, pool) = library(directory.path()).await;
    let (id, _) = book(&pool, "book").await;
    assert_eq!(fields(&pool, &id).await.0, "First");

    // A changed file is analyzed and refreshed again.
    let malformed = "<ComicInfo><Title>Second</Title><Year>soon</Year></ComicInfo>";
    write(directory.path(), "Series/book.cbz", &comic(Some(malformed)));
    rescan(&server, directory.path(), &library).await;
    assert_eq!(
        fields(&pool, &id).await.0,
        "First",
        "a malformed document changes nothing"
    );

    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"importComicInfoBook": false})),
        StatusCode::NO_CONTENT,
    )
    .await;
    write(
        directory.path(),
        "Series/book.cbz",
        &comic(Some(&title("Third title"))),
    );
    rescan(&server, directory.path(), &library).await;
    assert_eq!(
        fields(&pool, &id).await.0,
        "First",
        "the library imports no ComicInfo"
    );
    server.stop().await;
}

#[tokio::test]
async fn a_book_the_sort_renumbers_reads_its_number_again() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let numbered = "<ComicInfo><Number>5</Number></ComicInfo>";
    write(directory.path(), "Series/b.cbz", &comic(Some(numbered)));
    let (server, library, pool) = library(directory.path()).await;
    let (id, _) = book(&pool, "b").await;
    assert_eq!(
        (fields(&pool, &id).await.2, fields(&pool, &id).await.3),
        ("5".to_owned(), 5.0)
    );

    // `a` sorts first, which renumbers `b` to 2 until its refresh reads 5 again.
    write(directory.path(), "Series/a.cbz", &comic(None));
    rescan(&server, directory.path(), &library).await;
    let (first, _) = book(&pool, "a").await;
    assert_eq!(fields(&pool, &first).await.2, "1");
    let (_, _, number, number_sort, _, _) = fields(&pool, &id).await;
    assert_eq!((number.as_str(), number_sort), ("5", 5.0));
    server.stop().await;
}
