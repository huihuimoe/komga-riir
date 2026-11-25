use std::fs;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::scan::touch;
use super::{LIBRARIES, admin_json, create, route};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::{book, pages};
use crate::support::db::{self, content::scan_rows};
use crate::support::posters::{file, png};
use crate::support::tasks::{wait_for_scan_work, wait_for_tasks};
use crate::support::{TestServer, openapi};

const NOVEL: &str = "komga-archive/tests/fixtures/epub/The Incomplete Theft - Ralph Burke.epub";

/// Reader state and assets that library deletion may touch, and whether rows of the library's
/// books and series go with it (`BookLifecycle.deleteMany`, `SeriesLifecycle.deleteMany`).
const STATE: [(&str, &str, bool); 14] = [
    ("progress", "SELECT BOOK_ID FROM READ_PROGRESS", true),
    (
        "series progress",
        "SELECT SERIES_ID FROM READ_PROGRESS_SERIES",
        true,
    ),
    ("read list books", "SELECT BOOK_ID FROM READLIST_BOOK", true),
    (
        "collection series",
        "SELECT SERIES_ID FROM COLLECTION_SERIES",
        true,
    ),
    (
        "series posters",
        "SELECT DISTINCT SERIES_ID FROM THUMBNAIL_SERIES",
        true,
    ),
    (
        "book posters",
        "SELECT DISTINCT BOOK_ID FROM THUMBNAIL_BOOK",
        true,
    ),
    ("read lists", "SELECT ID FROM READLIST", false),
    (
        "read list posters",
        "SELECT READLIST_ID FROM THUMBNAIL_READLIST",
        false,
    ),
    ("collections", "SELECT ID FROM COLLECTION", false),
    (
        "collection posters",
        "SELECT COLLECTION_ID FROM THUMBNAIL_COLLECTION",
        false,
    ),
    ("known page hashes", "SELECT HASH FROM PAGE_HASH", false),
    (
        "page hash posters",
        "SELECT HASH FROM PAGE_HASH_THUMBNAIL",
        false,
    ),
    (
        "sync point books",
        "SELECT BOOK_ID FROM SYNC_POINT_BOOK",
        false,
    ),
    ("history", "SELECT BOOK_ID FROM HISTORICAL_EVENT", false),
];

/// Each [`STATE`] query's values, sorted.
async fn state(pool: &SqlitePool) -> Vec<(&'static str, Vec<String>)> {
    let mut state = Vec::new();
    for (name, sql, _) in STATE {
        let mut values: Vec<String> = sqlx::query_scalar(sql)
            .fetch_all(pool)
            .await
            .expect("Reader state");
        values.sort();
        state.push((name, values));
    }
    state
}

/// The ids of what `removed` and `kept` hold.
struct Content {
    removed_book: String,
    removed_series: String,
    page_book: String,
    kept_book: String,
    kept_series: String,
}

/// Two libraries, one with a hashed comic beside its novel, and the other with a novel.
async fn two_libraries(
    server: &TestServer,
    directory: &Path,
    pool: &SqlitePool,
) -> (Value, Content) {
    let novel = cbz::fixture(NOVEL);
    let comic = cbz::zip(&[("1.png", STORED, &png(8, 8))]);
    let files = [
        ("removed/R/removed.epub", novel.clone()),
        ("removed/R/page.cbz", comic),
        ("kept/K/kept.epub", novel),
    ];
    for (path, content) in files {
        let path = directory.join(path);
        fs::create_dir_all(path.parent().expect("Book directory")).expect("Book directory");
        fs::write(path, content).expect("Book file");
    }
    create(server, "Kept", &directory.join("kept")).await;
    let removed = create(server, "Removed", &directory.join("removed")).await;
    let hash_pages = json!({"hashPages": true});
    admin_json(
        server,
        Method::PATCH,
        &route(&removed),
        Some(&hash_pages),
        StatusCode::NO_CONTENT,
    )
    .await;
    wait_for_scan_work(directory).await;
    let (removed_book, removed_series) = book(pool, "removed").await;
    let (kept_book, kept_series) = book(pool, "kept").await;
    let content = Content {
        removed_book,
        removed_series,
        page_book: book(pool, "page").await.0,
        kept_book,
        kept_series,
    };
    (removed, content)
}

/// Progress, a read list and a collection over both libraries with posters, a known page hash,
/// a Kobo sync point, and the history of a deleted book file.
async fn reader_state(server: &TestServer, directory: &Path, pool: &SqlitePool, c: &Content) {
    for book in [&c.removed_book, &c.kept_book] {
        let path = format!("/api/v1/books/{book}/read-progress");
        let progress = Some(json!({"completed": true}));
        admin_json(
            server,
            Method::PATCH,
            &path,
            progress.as_ref(),
            StatusCode::NO_CONTENT,
        )
        .await;
    }
    sets_with_posters(server, c).await;
    let hash = pages(pool, &c.page_book).await[0].4.clone();
    let known = json!({"hash": hash, "action": "IGNORE"});
    admin_json(
        server,
        Method::PUT,
        "/api/v1/page-hashes",
        Some(&known),
        StatusCode::ACCEPTED,
    )
    .await;
    let key = admin_json(
        server,
        Method::POST,
        "/api/v2/users/me/api-keys",
        Some(&json!({"comment": "kobo"})),
        StatusCode::OK,
    )
    .await;
    let key = key["key"].as_str().expect("Plaintext key");
    let sync = server
        .request(Method::GET, &format!("/kobo/{key}/v1/library/sync"))
        .send()
        .await;
    assert_eq!(sync.expect("Kobo sync").status(), StatusCode::OK);
    let path = format!("/api/v1/books/{}/file", c.page_book);
    admin_json(server, Method::DELETE, &path, None, StatusCode::ACCEPTED).await;
    // Everything queued has run; none fails.
    wait_for_tasks(directory, |rows| rows.is_empty()).await;
}

/// A read list and a collection over both libraries, and posters for them and both series.
async fn sets_with_posters(server: &TestServer, c: &Content) {
    let read_list = json!({"name": "Mixed", "bookIds": [c.removed_book, c.kept_book]});
    let read_list = admin_json(
        server,
        Method::POST,
        "/api/v1/readlists",
        Some(&read_list),
        StatusCode::OK,
    )
    .await;
    let collection =
        json!({"name": "Mixed", "ordered": false, "seriesIds": [c.removed_series, c.kept_series]});
    let collection = admin_json(
        server,
        Method::POST,
        "/api/v1/collections",
        Some(&collection),
        StatusCode::OK,
    )
    .await;
    for owner in [
        format!(
            "readlists/{}",
            read_list["id"].as_str().expect("Read list ID")
        ),
        format!(
            "collections/{}",
            collection["id"].as_str().expect("Collection ID")
        ),
        format!("series/{}", c.removed_series),
        format!("series/{}", c.kept_series),
    ] {
        let path = format!("/api/v1/{owner}/thumbnails");
        let response = server
            .admin(Method::POST, &path)
            .multipart(file(png(4, 4)))
            .send()
            .await;
        assert_eq!(
            response.expect("Poster upload").status(),
            StatusCode::OK,
            "{path}"
        );
    }
}

#[tokio::test]
async fn delete_cleans_reader_state_and_keeps_non_library_assets() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let pool = db::main_pool(directory.path()).await;
    let (removed, c) = two_libraries(&server, directory.path(), &pool).await;
    reader_state(&server, directory.path(), &pool, &c).await;
    let before = state(&pool).await;
    let removed_ids = [&c.removed_book, &c.removed_series, &c.page_book];
    let expected: Vec<_> = before
        .iter()
        .zip(STATE)
        .map(|((name, values), (_, _, owned))| {
            let kept = values
                .iter()
                .filter(|value| !owned || !removed_ids.contains(value));
            let kept: Vec<String> = kept.cloned().collect();
            assert!(!kept.is_empty(), "{name} keeps something");
            assert_eq!(
                owned,
                kept != *values,
                "{name} holds the library: {values:?}"
            );
            (*name, kept)
        })
        .collect();

    admin_json(
        &server,
        Method::DELETE,
        &route(&removed),
        None,
        StatusCode::NO_CONTENT,
    )
    .await;
    assert_eq!(state(&pool).await, expected);
    pool.close().await;
    server.stop().await;
}

#[tokio::test]
async fn delete_library_rolls_back_and_keeps_source_files_and_other_libraries() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let removed_root = directory.path().join("removed");
    let kept_root = directory.path().join("kept");
    for file in ["Series K/k.cbz", "Series K/cover.jpg"] {
        touch(&kept_root.join(file));
    }
    let kept = create(&server, "Kept", &kept_root).await;
    wait_for_scan_work(directory.path()).await;
    let pool = db::main_pool(directory.path()).await;
    let kept_content = scan_rows(&pool).await;
    for file in ["Series R/r.cbz", "Series R/series.json"] {
        touch(&removed_root.join(file));
    }
    let source = removed_root.join("source.cbz");
    std::fs::write(&source, b"source archive bytes").expect("Source file");
    let removed = create(&server, "Removed", &removed_root).await;
    admin_json(
        &server,
        Method::PATCH,
        &route(&removed),
        Some(&json!({"scanDirectoryExclusions": ["private*"]})),
        StatusCode::NO_CONTENT,
    )
    .await;
    // Creation and the exclusion change queue scans.
    wait_for_scan_work(directory.path()).await;
    admin_json(
        &server,
        Method::POST,
        "/api/v2/users",
        Some(&json!({
            "email": "reader@example.com", "password": "readerpass",
            "sharedLibraries": {"all": false, "libraryIds": [removed["id"], kept["id"]]}
        })),
        StatusCode::CREATED,
    )
    .await;
    let all_content = scan_rows(&pool).await;
    // Two series and books with their metadata, media, aggregations, and `series.json`.
    assert_eq!(
        all_content.len() - kept_content.len(),
        13,
        "{all_content:?}"
    );
    db::inject_failure(&pool, "BEFORE DELETE ON LIBRARY").await;
    admin_json(
        &server,
        Method::DELETE,
        &route(&removed),
        None,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    let library = admin_json(&server, Method::GET, &route(&removed), None, StatusCode::OK).await;
    assert_eq!(library["scanDirectoryExclusions"], json!(["private*"]));
    assert_reader_shares(&server, &[removed["id"].clone(), kept["id"].clone()]).await;
    assert_eq!(
        scan_rows(&pool).await,
        all_content,
        "a failed deletion keeps the content"
    );
    db::remove_failure(&pool).await;
    admin_json(
        &server,
        Method::DELETE,
        &route(&removed),
        None,
        StatusCode::NO_CONTENT,
    )
    .await;
    assert_reader_shares(&server, &[kept["id"].clone()]).await;
    assert_eq!(scan_rows(&pool).await, kept_content);
    assert_eq!(
        std::fs::read(source).expect("Source survives"),
        b"source archive bytes"
    );
    assert_eq!(
        admin_json(&server, Method::GET, LIBRARIES, None, StatusCode::OK).await,
        json!([kept])
    );
    admin_json(
        &server,
        Method::DELETE,
        &route(&removed),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
    pool.close().await;
    server.stop().await;
}

async fn assert_reader_shares(server: &TestServer, expected: &[Value]) {
    let response = server
        .request(Method::GET, "/api/v2/users/me")
        .basic_auth("reader@example.com", Some("readerpass"))
        .send()
        .await
        .expect("Read library sharing");
    assert_eq!(response.status(), StatusCode::OK);
    let user = openapi::checked(Method::GET, response).await;
    let mut actual = user["sharedLibrariesIds"]
        .as_array()
        .expect("Shared libraries")
        .clone();
    actual.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    let mut expected = expected.to_vec();
    expected.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    assert_eq!(
        actual, expected,
        "Deletion must preserve unrelated library sharing"
    );
}
