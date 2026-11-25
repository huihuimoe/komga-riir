//! `SearchIndexLifecycle.consumeEvents` for books and series: the index follows scans, analysis,
//! metadata refreshes, aggregation and edits, book counts, soft deletion, and the trash.

use std::fs;
use std::path::Path;

use komga_types::SearchEntity;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{comic, library, rescan, rescan_deep, write};
use crate::libraries::{admin_json, route};
use crate::support::TestServer;
use crate::support::db::content::book;
use crate::support::search::{hits_in, open, wait_for_index};
use crate::support::tasks::{wait_for_scan_work, wait_for_tasks};

const BATMAN: &str = "<ComicInfo><Title>Batman Year One</Title><Writer>Frank Miller</Writer>\
    <Year>1987</Year><Tags>noir</Tags></ComicInfo>";
const ONESHOT: &str = "<ComicInfo><Title>Watchmen</Title><Publisher>DC</Publisher>\
    <Genre>Superhero</Genre></ComicInfo>";

async fn patch(server: &TestServer, path: &str, body: &Value) {
    admin_json(
        server,
        Method::PATCH,
        path,
        Some(body),
        StatusCode::NO_CONTENT,
    )
    .await;
}

async fn empty_trash(server: &TestServer, directory: &Path, library: &Value) {
    let path = format!("{}/empty-trash", route(library));
    admin_json(server, Method::POST, &path, None, StatusCode::ACCEPTED).await;
    wait_for_tasks(directory, |rows| {
        !rows.iter().any(|row| row.2.ends_with("$EmptyTrash"))
    })
    .await;
}

#[tokio::test]
async fn books_are_indexed_as_the_database_holds_them() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let dir = directory.path();
    write(dir, "series/batman.cbz", &comic(Some(BATMAN)));
    write(dir, "series/robin.cbz", &comic(None));
    write(dir, "series/joker.cbz", &comic(None));
    write(dir, "_oneshots/one.cbz", &comic(Some(ONESHOT)));
    let (server, library, pool) = library(dir).await;
    // Each change below follows the indexing of its book, so that only the change itself can
    // update the document. The book scanned in a plain series is restored into its new oneshot
    // series.
    wait_for_index(&pool).await;
    let body = json!({"oneshotsDirectory": "_oneshots"});
    patch(&server, &route(&library), &body).await;
    rescan_deep(&server, dir, &library).await;
    let (batman, _) = book(&pool, "batman").await;
    let (robin, _) = book(&pool, "robin").await;
    let (one, one_series) = book(&pool, "one").await;
    wait_for_index(&pool).await;

    let body = json!({"title": "Nightwing", "titleLock": true});
    patch(&server, &format!("/api/v1/books/{robin}/metadata"), &body).await;
    let body = json!({"publisher": "Vertigo", "publisherLock": true});
    patch(
        &server,
        &format!("/api/v1/series/{one_series}/metadata"),
        &body,
    )
    .await;
    fs::remove_file(dir.join("comics/series/joker.cbz")).expect("Remove joker");
    rescan(&server, dir, &library).await;
    wait_for_index(&pool).await;
    empty_trash(&server, dir, &library).await;
    fs::remove_file(dir.join("comics/series/batman.cbz")).expect("Remove batman");
    rescan(&server, dir, &library).await;
    wait_for_scan_work(dir).await;
    wait_for_index(&pool).await;
    server.stop().await;

    let index = open(dir);
    let found = |query: &str| hits_in(&index, SearchEntity::Book, query);
    let mut all = vec![batman.clone(), one.clone(), robin.clone()];
    all.sort();
    assert_eq!(
        found("*:*"),
        all,
        "the emptied trash and the replaced book are gone"
    );
    assert_eq!(found("status:ready"), all, "every book is analyzed");
    for (query, expected) in [
        ("batman", &batman),
        ("writer:miller", &batman),
        ("author:frank", &batman),
        ("release_date:1987", &batman),
        ("tag:noir", &batman),
        ("deleted:true", &batman),
        ("nightwing", &robin),
        ("watchmen", &one),
        ("oneshot:true", &one),
        ("publisher:vertigo", &one),
        ("genre:superhero", &one),
        ("complete:true", &one),
    ] {
        assert_eq!(found(query), [expected.as_str()], "{query}");
    }
    for query in ["robin", "joker", "publisher:dc", "isbn:*"] {
        assert!(found(query).is_empty(), "{query}");
    }
}

#[tokio::test]
async fn series_are_indexed_as_the_database_holds_them() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let dir = directory.path();
    write(dir, "knight/batman.cbz", &comic(Some(BATMAN)));
    write(dir, "knight/robin.cbz", &comic(None));
    write(dir, "knight/joker.cbz", &comic(None));
    write(dir, "gone/x.cbz", &comic(None));
    write(dir, "trashed/y.cbz", &comic(None));
    write(dir, "solo/z.cbz", &comic(None));
    write(dir, "_oneshots/one.cbz", &comic(Some(ONESHOT)));
    let (server, library, pool) = library(dir).await;
    // As for books, each change follows the indexing of its series. The plain series scanned
    // first in `_oneshots` is restored into the oneshot series, then deleted.
    wait_for_index(&pool).await;
    let body = json!({"oneshotsDirectory": "_oneshots"});
    patch(&server, &route(&library), &body).await;
    rescan_deep(&server, dir, &library).await;
    let (_, knight) = book(&pool, "batman").await;
    let (_, gone) = book(&pool, "x").await;
    let (_, one) = book(&pool, "one").await;
    let (z, solo) = book(&pool, "z").await;
    wait_for_index(&pool).await;

    // Only the aggregation the book edit queues changes the series.
    let body = json!({"authors": [{"name": "Alan Moore", "role": "writer"}]});
    patch(&server, &format!("/api/v1/books/{z}/metadata"), &body).await;
    let body = json!({
        "alternateTitles": [{"label": "en", "title": "Ozymandias"}],
        "alternateTitlesLock": true,
    });
    patch(&server, &format!("/api/v1/series/{one}/metadata"), &body).await;
    let body = json!({
        "tags": ["gotham"],
        "tagsLock": true,
        "totalBookCount": 2,
        "totalBookCountLock": true,
    });
    patch(&server, &format!("/api/v1/series/{knight}/metadata"), &body).await;
    // The soft-deleted book still counts until the trash is emptied.
    fs::remove_file(dir.join("comics/knight/robin.cbz")).expect("Remove robin");
    fs::remove_dir_all(dir.join("comics/trashed")).expect("Remove trashed");
    rescan(&server, dir, &library).await;
    wait_for_index(&pool).await;
    empty_trash(&server, dir, &library).await;
    fs::remove_dir_all(dir.join("comics/gone")).expect("Remove gone");
    rescan(&server, dir, &library).await;
    wait_for_scan_work(dir).await;
    wait_for_index(&pool).await;
    server.stop().await;

    let index = open(dir);
    let found = |query: &str| hits_in(&index, SearchEntity::Series, query);
    let mut all = vec![gone.clone(), knight.clone(), one.clone(), solo.clone()];
    all.sort();
    assert_eq!(
        found("*:*"),
        all,
        "the emptied trash and the restored series are gone"
    );
    let mut complete = vec![knight.clone(), one.clone()];
    complete.sort();
    assert_eq!(found("complete:true"), complete);
    for (query, expected) in [
        ("knight", &knight),
        ("writer:miller", &knight),
        ("writer:moore", &solo),
        ("book_tag:noir", &knight),
        ("tag:noir", &knight),
        ("series_tag:gotham", &knight),
        ("release_date:1987", &knight),
        ("book_count:2", &knight),
        ("deleted:true", &gone),
        ("oneshot:true", &one),
        ("ozymandias", &one),
        ("publisher:dc", &one),
        ("genre:superhero", &one),
    ] {
        assert_eq!(found(query), [expected.as_str()], "{query}");
    }
    assert!(found("book_count:3").is_empty());
}
