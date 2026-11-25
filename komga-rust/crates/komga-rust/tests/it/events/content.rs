//! Series and book events, from scans, analysis, edits, and deletions.

use std::fs;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{EVENTS, EventStream};
use crate::libraries;
use crate::read_lists::{PASSWORD, create_user, rate};
use crate::support::TestServer;
use crate::support::cbz::{self, STORED};
use crate::support::db::{self, content::book};
use crate::support::tasks::{wait_for_scan_work, wait_for_tasks};

fn series(name: &str, series_id: &str, library_id: &str) -> (String, Value) {
    let data = json!({"seriesId": series_id, "libraryId": library_id});
    (name.to_owned(), data)
}

fn book_event(
    name: &str,
    (book_id, series_id): &(String, String),
    library_id: &str,
) -> (String, Value) {
    let data = json!({"bookId": book_id, "seriesId": series_id, "libraryId": library_id});
    (name.to_owned(), data)
}

/// Reads events until each of `expected` came, in any order.
async fn expect_all(stream: &mut EventStream, mut expected: Vec<(String, Value)>) {
    while !expected.is_empty() {
        let event = stream.next_change().await;
        expected.retain(|wanted| *wanted != event);
    }
}

async fn admin_post(server: &TestServer, path: &str) {
    let response = server
        .admin(Method::POST, path)
        .send()
        .await
        .expect("Request");
    assert_eq!(response.status(), StatusCode::ACCEPTED, "{path}");
}

#[tokio::test]
async fn content_changes_reach_those_who_may_see_the_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let page = cbz::zip(&[(
        "page.png",
        STORED,
        &cbz::fixture("komga-image/tests/fixtures/photo.png"),
    )]);
    for path in ["lib/SA/a.cbz", "lib/SB/b.cbz"] {
        let path = directory.path().join(path);
        fs::create_dir_all(path.parent().expect("Series directory")).expect("Series directory");
        fs::write(path, &page).expect("Book file");
    }
    let server = TestServer::start_claimed(directory.path()).await;
    let mut admin = EventStream::open(server.admin(Method::GET, EVENTS)).await;
    let library = libraries::create(&server, "L", &directory.path().join("lib")).await;
    let library_id = library["id"].as_str().expect("Library ID").to_owned();
    wait_for_scan_work(directory.path()).await;
    let pool = db::main_pool(directory.path()).await;
    let (a, b) = (book(&pool, "a").await, book(&pool, "b").await);
    let lib = library_id.as_str();
    let mut created = Vec::new();
    for book in [&a, &b] {
        created.push(series("SeriesAdded", &book.1, lib));
        created.push(book_event("BookAdded", book, lib));
        // Analysis, then the series aggregating its books' metadata.
        created.push(book_event("BookChanged", book, lib));
        created.push(series("SeriesChanged", &book.1, lib));
    }
    expect_all(&mut admin, created).await;

    // Only series rated 12 or under are visible to the reader.
    let access = json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}});
    create_user(&server, "reader@example.org", access).await;
    let reader = server
        .request(Method::GET, EVENTS)
        .basic_auth("reader@example.org", Some(PASSWORD));
    let mut reader = EventStream::open(reader).await;
    rate(&server, &b.1, 18).await;
    rate(&server, &a.1, 10).await;
    assert_eq!(
        reader.next().await,
        Some(series("SeriesChanged", &a.1, lib))
    );

    // A book gone from disk is soft deleted with its series, then emptying the trash deletes them.
    fs::remove_file(directory.path().join("lib/SB/b.cbz")).expect("Remove book");
    admin_post(&server, &format!("{}/scan", libraries::route(&library))).await;
    let trashed = vec![
        book_event("BookChanged", &b, lib),
        series("SeriesChanged", &b.1, lib),
    ];
    expect_all(&mut admin, trashed).await;
    wait_for_scan_work(directory.path()).await;
    admin_post(
        &server,
        &format!("{}/empty-trash", libraries::route(&library)),
    )
    .await;
    let emptied = vec![
        book_event("BookDeleted", &b, lib),
        series("SeriesDeleted", &b.1, lib),
    ];
    expect_all(&mut admin, emptied).await;
    wait_for_scan_work(directory.path()).await;
    wait_for_tasks(directory.path(), |rows| {
        !rows.iter().any(|row| row.2.ends_with("$EmptyTrash"))
    })
    .await;

    // Deleting the library deletes its content first.
    let response = server
        .admin(Method::DELETE, &libraries::route(&library))
        .send()
        .await
        .expect("Delete library");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let deleted = [
        book_event("BookDeleted", &a, lib),
        series("SeriesDeleted", &a.1, lib),
        ("LibraryDeleted".to_owned(), json!({"libraryId": lib})),
    ];
    for event in deleted.clone() {
        assert_eq!(admin.next_change().await, event);
    }
    // The reader saw nothing of series B but its deletion, whose rating is gone by then.
    let trashed_b = [
        book_event("BookDeleted", &b, lib),
        series("SeriesDeleted", &b.1, lib),
    ];
    for event in trashed_b.into_iter().chain(deleted) {
        assert_eq!(reader.next().await, Some(event));
    }
    server.stop().await;
}
