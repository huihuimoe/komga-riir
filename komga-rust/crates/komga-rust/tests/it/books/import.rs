//! `POST /api/v1/books/import`: books from outside every library copied, moved or linked into a
//! series.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::{JPEG, library_with_books};
use crate::events::{EVENTS, EventStream};
use crate::libraries::admin_json;
use crate::read_lists::{PASSWORD, create_user};
use crate::support::cbz::{self, STORED};
use crate::support::tasks::{wait_for_scan_work, wait_for_tasks};
use crate::support::{TestServer, db};

const ROUTE: &str = "/api/v1/books/import";
const IMPORT_BOOK: &str = "org.gotson.komga.application.tasks.Task$ImportBook";

fn comic() -> Vec<u8> {
    cbz::zip(&[("1.jpg", STORED, &cbz::fixture(JPEG))])
}

/// A library holding the series `Watchmen` and its ID, an admin event stream, and a folder
/// outside the library holding `files`.
async fn fixture(
    directory: &Path,
    files: &[&str],
) -> (TestServer, SqlitePool, String, EventStream) {
    let (server, _, pool) =
        library_with_books(directory, &[("Watchmen/Watchmen 1.cbz", comic())]).await;
    let series_id = sqlx::query_scalar("SELECT ID FROM SERIES")
        .fetch_one(&pool)
        .await
        .expect("Series");
    let incoming = directory.join("incoming");
    fs::create_dir(&incoming).expect("Incoming folder");
    for file in files {
        fs::write(incoming.join(file), comic()).expect("Incoming file");
    }
    let events = EventStream::open(server.admin(Method::GET, EVENTS)).await;
    (server, pool, series_id, events)
}

/// Imports `book` and returns the `BookImported` event once the import and its follow-ups ran.
async fn import(
    server: &TestServer,
    directory: &Path,
    events: &mut EventStream,
    copy_mode: &str,
    book: Value,
) -> Value {
    let response = server
        .admin(Method::POST, ROUTE)
        .json(&json!({"copyMode": copy_mode, "books": [book]}))
        .send()
        .await
        .expect("Import request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    wait_for_tasks(directory, |rows| {
        rows.iter()
            .filter(|row| row.2 == IMPORT_BOOK)
            .all(|row| row.3.contains("\"eraFailed\""))
    })
    .await;
    wait_for_scan_work(directory).await;
    loop {
        let (name, event) = events.next().await.expect("An open stream");
        if name == "BookImported" {
            return event;
        }
    }
}

async fn series_books(pool: &SqlitePool, series_id: &str) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT b.NAME, m.STATUS FROM BOOK b JOIN MEDIA m ON m.BOOK_ID = b.ID \
         WHERE b.SERIES_ID = ? ORDER BY b.NAME",
    )
    .bind(series_id)
    .fetch_all(pool)
    .await
    .expect("Series books")
}

#[cfg(unix)]
#[tokio::test]
async fn books_are_copied_moved_and_linked_into_their_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let files = ["Watchmen 2.cbz", "Watchmen 2.jpg", "w3.cbz", "w4.cbz"];
    let (server, pool, series_id, mut events) = fixture(directory.path(), &files).await;
    let incoming = directory.path().join("incoming");
    let series = directory.path().join("comics/Watchmen");
    let book = |file: &str, name: Option<&str>| json!({"sourceFile": incoming.join(file), "seriesId": series_id, "destinationName": name});

    let copied = import(
        &server,
        directory.path(),
        &mut events,
        "COPY",
        book("Watchmen 2.cbz", None),
    )
    .await;
    assert_eq!(copied["success"], true, "{copied}");
    assert!(copied["bookId"].is_string());
    assert!(incoming.join("Watchmen 2.cbz").exists() && incoming.join("Watchmen 2.jpg").exists());
    assert!(series.join("Watchmen 2.cbz").exists() && series.join("Watchmen 2.jpg").exists());

    let moved = import(
        &server,
        directory.path(),
        &mut events,
        "MOVE",
        book("w3.cbz", Some("Watchmen 3")),
    )
    .await;
    assert_eq!(moved["success"], true, "{moved}");
    assert!(!incoming.join("w3.cbz").exists());
    assert!(series.join("Watchmen 3.cbz").exists());

    let linked = import(
        &server,
        directory.path(),
        &mut events,
        "HARDLINK",
        book("w4.cbz", None),
    )
    .await;
    assert_eq!(linked["success"], true, "{linked}");
    let inode = |path: &Path| fs::metadata(path).expect("Linked file").ino();
    assert_eq!(
        inode(&incoming.join("w4.cbz")),
        inode(&series.join("w4.cbz"))
    );

    let ready = |name: &str| (name.to_owned(), "READY".to_owned());
    assert_eq!(
        series_books(&pool, &series_id).await,
        [
            ready("Watchmen 1"),
            ready("Watchmen 2"),
            ready("Watchmen 3"),
            ready("w4")
        ]
    );
    let sidecar: String =
        sqlx::query_scalar("SELECT URL FROM SIDECAR WHERE URL LIKE '%Watchmen%2.jpg'")
            .fetch_one(&pool)
            .await
            .expect("Imported sidecar");
    assert!(sidecar.starts_with("file:"), "{sidecar}");
}

#[tokio::test]
async fn refused_imports_are_told_to_admins_and_leave_files_alone() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool, series_id, mut events) =
        fixture(directory.path(), &["Watchmen 1.cbz"]).await;
    let series = directory.path().join("comics/Watchmen");
    let library_file = series.join("Watchmen 1.cbz");
    let before = fs::read(&library_file).expect("Library file");
    for (source, code) in [
        (directory.path().join("missing.cbz"), "ERR_1018"),
        (library_file.clone(), "ERR_1019"),
        (directory.path().join("incoming/Watchmen 1.cbz"), "ERR_1021"),
    ] {
        let book = json!({"sourceFile": source, "seriesId": series_id});
        let event = import(&server, directory.path(), &mut events, "MOVE", book).await;
        let expected =
            json!({"bookId": null, "sourceFile": source, "success": false, "message": code});
        assert_eq!(event, expected);
    }
    assert!(directory.path().join("incoming/Watchmen 1.cbz").exists());
    assert_eq!(fs::read(&library_file).expect("Library file"), before);
    assert_eq!(fs::read_dir(&series).expect("Series folder").count(), 1);
    assert_eq!(series_books(&pool, &series_id).await.len(), 1);

    create_user(&server, "reader@example.org", json!({})).await;
    let response = server
        .request(Method::POST, ROUTE)
        .basic_auth("reader@example.org", Some(PASSWORD))
        .json(&json!({"copyMode": "COPY", "books": []}))
        .send()
        .await
        .expect("Import request");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

async fn scalar(pool: &SqlitePool, sql: &'static str, book_id: &str) -> Option<String> {
    sqlx::query_scalar(sql)
        .bind(book_id)
        .fetch_optional(pool)
        .await
        .expect("Query")
}

#[tokio::test]
async fn an_upgrade_hands_the_book_over_and_is_not_replayed() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let files = ["Watchmen 2.cbz", "Watchmen 2.jpg", "w2.cbz"];
    let (server, pool, series_id, mut events) = fixture(directory.path(), &files).await;
    let incoming = directory.path().join("incoming");
    let series = directory.path().join("comics/Watchmen");
    let two_pages = cbz::zip(&[
        ("1.jpg", STORED, &cbz::fixture(JPEG)),
        ("2.jpg", STORED, &cbz::fixture(JPEG)),
    ]);
    fs::write(incoming.join("w2.cbz"), &two_pages).expect("Upgrade file");
    let book = json!({"sourceFile": incoming.join("Watchmen 2.cbz"), "seriesId": series_id});
    let imported = import(&server, directory.path(), &mut events, "COPY", book).await;
    let old_id = imported["bookId"]
        .as_str()
        .expect("Imported book")
        .to_owned();
    for (path, body) in [
        ("metadata", json!({"summary": "Kept"})),
        ("read-progress", json!({"completed": true})),
    ] {
        let response = server
            .admin(Method::PATCH, &format!("/api/v1/books/{old_id}/{path}"))
            .json(&body)
            .send()
            .await
            .expect("Book update");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    let upgrade = json!({"sourceFile": incoming.join("w2.cbz"), "seriesId": series_id, "destinationName": "Watchmen 2", "upgradeBookId": old_id});
    let upgraded = import(
        &server,
        directory.path(),
        &mut events,
        "MOVE",
        upgrade.clone(),
    )
    .await;
    assert_eq!(upgraded["success"], true, "{upgraded}");
    let new_id = upgraded["bookId"].as_str().expect("Upgraded book");
    assert_ne!(new_id, old_id);
    assert_eq!(
        fs::read(series.join("Watchmen 2.cbz")).expect("Upgraded file"),
        two_pages
    );
    assert!(!incoming.join("w2.cbz").exists() && !series.join("Watchmen 2.jpg").exists());
    let ready = |name: &str| (name.to_owned(), "READY".to_owned());
    assert_eq!(
        series_books(&pool, &series_id).await,
        [ready("Watchmen 1"), ready("Watchmen 2")]
    );
    let summary = "SELECT SUMMARY FROM BOOK_METADATA WHERE BOOK_ID = ?";
    assert_eq!(
        scalar(&pool, summary, new_id).await.as_deref(),
        Some("Kept")
    );
    let progress = "SELECT CAST(COMPLETED AS TEXT) FROM READ_PROGRESS WHERE BOOK_ID = ?";
    assert_eq!(scalar(&pool, progress, new_id).await.as_deref(), Some("1"));
    let old_rows =
        "SELECT ID FROM BOOK WHERE ID = ? UNION SELECT URL FROM SIDECAR WHERE URL LIKE '%2.jpg'";
    assert_eq!(scalar(&pool, old_rows, &old_id).await, None);

    // Replayed, the upgraded book is gone and the import meets its own file.
    fs::write(incoming.join("w2.cbz"), &two_pages).expect("Upgrade file");
    let replayed = import(&server, directory.path(), &mut events, "MOVE", upgrade).await;
    assert_eq!(replayed["message"], "ERR_1021");
    assert_eq!(series_books(&pool, &series_id).await.len(), 2);
}

#[tokio::test]
async fn a_oneshot_is_only_upgraded_and_follows_its_new_file() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let oneshots = directory.path().join("comics/_oneshots");
    fs::create_dir_all(&oneshots).expect("Oneshots folder");
    fs::write(oneshots.join("One.cbz"), comic()).expect("Oneshot");
    let incoming = directory.path().join("incoming");
    fs::create_dir(&incoming).expect("Incoming folder");
    fs::write(incoming.join("One v2.cbz"), comic()).expect("Incoming file");
    let body = json!({"name": "Comics", "root": directory.path().join("comics"), "oneshotsDirectory": "_oneshots"});
    admin_json(
        &server,
        Method::POST,
        "/api/v1/libraries",
        Some(&body),
        StatusCode::OK,
    )
    .await;
    wait_for_scan_work(directory.path()).await;
    let pool = db::main_pool(directory.path()).await;
    let (book_id, series_id): (String, String) = sqlx::query_as("SELECT ID, SERIES_ID FROM BOOK")
        .fetch_one(&pool)
        .await
        .expect("Oneshot");
    let mut events = EventStream::open(server.admin(Method::GET, EVENTS)).await;
    let book = json!({"sourceFile": incoming.join("One v2.cbz"), "seriesId": series_id});

    let refused = import(&server, directory.path(), &mut events, "COPY", book.clone()).await;
    assert_eq!(refused["success"], false, "{refused}");
    let mut upgrade = book;
    upgrade["upgradeBookId"] = json!(book_id);
    let upgraded = import(&server, directory.path(), &mut events, "COPY", upgrade).await;
    assert_eq!(upgraded["success"], true, "{upgraded}");
    assert!(!oneshots.join("One.cbz").exists() && oneshots.join("One v2.cbz").exists());
    let (url, oneshot): (String, bool) =
        sqlx::query_as("SELECT s.URL, b.ONESHOT FROM SERIES s JOIN BOOK b ON b.SERIES_ID = s.ID")
            .fetch_one(&pool)
            .await
            .expect("Upgraded oneshot");
    assert!(url.ends_with("One%20v2.cbz") && oneshot, "{url}");
}
