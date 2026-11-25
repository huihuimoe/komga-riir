//! `RefreshBookMetadata` adding books to the read lists of their `ComicInfo.xml`.

use std::fs;
use std::path::Path;

use komga_types::SearchEntity;
use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use super::{assert_no_failed_task, comic, library, rescan, write};
use crate::libraries::{admin_json, route};
use crate::support::TestServer;
use crate::support::db::content::book;
use crate::support::search::{hits, wait_for_index};
use crate::support::tasks::wait_for_scan_work;

/// `(READLIST.NAME, BOOK_COUNT, BOOK.NAME, NUMBER)` by list name and number.
type Member = (String, i64, String, i64);

async fn members(pool: &SqlitePool) -> Vec<Member> {
    sqlx::query_as(
        "SELECT r.NAME, r.BOOK_COUNT, b.NAME, rb.NUMBER FROM READLIST r \
         JOIN READLIST_BOOK rb ON rb.READLIST_ID = r.ID JOIN BOOK b ON b.ID = rb.BOOK_ID \
         ORDER BY r.NAME, rb.NUMBER",
    )
    .fetch_all(pool)
    .await
    .expect("Read list members")
}

fn member(list: &str, count: i64, book: &str, number: i64) -> Member {
    (list.to_owned(), count, book.to_owned(), number)
}

async fn analyze(server: &TestServer, directory: &Path, book_id: &str) {
    let response = server
        .admin(Method::POST, &format!("/api/v1/books/{book_id}/analyze"))
        .send()
        .await
        .expect("Analyze request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_no_failed_task(wait_for_scan_work(directory).await);
}

#[tokio::test]
async fn story_arcs_add_each_book_once_after_the_taken_numbers() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let first = "<ComicInfo><AlternateSeries>Alt</AlternateSeries>\
                 <StoryArc>Arc</StoryArc><StoryArcNumber>2</StoryArcNumber></ComicInfo>";
    write(directory.path(), "Series/a.cbz", &comic(Some(first)));
    let (server, library, pool) = library(directory.path()).await;

    // `arc` names the same list; its number 2 is taken, so `b` goes after the last book.
    let second = "<ComicInfo><StoryArc>arc</StoryArc><StoryArcNumber>2</StoryArcNumber>\
                  </ComicInfo>";
    write(directory.path(), "Series/b.cbz", &comic(Some(second)));
    rescan(&server, directory.path(), &library).await;
    let expected = [
        member("Alt", 1, "a", 0),
        member("Arc", 2, "a", 2),
        member("Arc", 2, "b", 3),
    ];
    assert_eq!(members(&pool).await, expected);

    for name in ["a", "b"] {
        let (id, _) = book(&pool, name).await;
        analyze(&server, directory.path(), &id).await;
    }
    assert_eq!(
        members(&pool).await,
        expected,
        "refreshing again adds no book twice"
    );
    wait_for_index(&pool).await;
    server.stop().await;
    let found = |query| hits(directory.path(), SearchEntity::ReadList, query).len();
    assert_eq!([found("*:*"), found("alt"), found("arc")], [2, 1, 1]);
}

#[tokio::test]
async fn a_disabled_read_list_import_creates_no_read_list() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    fs::create_dir(directory.path().join("comics")).expect("Library root");
    let (server, library, pool) = library(directory.path()).await;
    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"importComicInfoReadList": false})),
        StatusCode::NO_CONTENT,
    )
    .await;
    let arc = "<ComicInfo><Title>Kept</Title><StoryArc>Arc</StoryArc></ComicInfo>";
    write(directory.path(), "Series/a.cbz", &comic(Some(arc)));
    rescan(&server, directory.path(), &library).await;

    let (id, _) = book(&pool, "a").await;
    let title: String = sqlx::query_scalar("SELECT TITLE FROM BOOK_METADATA WHERE BOOK_ID = ?")
        .bind(&id)
        .fetch_one(&pool)
        .await
        .expect("Book title");
    assert_eq!(title, "Kept", "the book metadata is still imported");
    assert_eq!(members(&pool).await, []);
    server.stop().await;
}
