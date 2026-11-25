//! Book and series files deleted on request (`BookLifecycle.deleteBookFiles`,
//! `SeriesLifecycle.deleteSeriesFiles`).

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use crate::books::reading::access::create_reader;
use crate::books::reading::fetch;
use crate::books::thumbnails::cover;
use crate::books::{PNG, library_with_books};
use crate::support::TestServer;
use crate::support::cbz::{self, STORED};
use crate::support::db;
use crate::support::db::content::{book, deletion_dates};
use crate::support::tasks::{TaskRow, wait_for_scan_work, wait_for_tasks};

/// `a` with a sidecar cover and `b` share `A`, `c` is alone in `C` and `d` in the library root.
pub(crate) async fn library(directory: &Path) -> (TestServer, SqlitePool) {
    let book = cbz::zip(&[("1.png", STORED, &cover())]);
    let books = [
        ("A/a.cbz", book.clone()),
        ("A/a.png", cbz::fixture(PNG)),
        ("A/b.cbz", book.clone()),
        ("C/c.cbz", book.clone()),
        ("d.cbz", book),
    ];
    let (server, _, pool) = library_with_books(directory, &books).await;
    (server, pool)
}

/// Series `S` with its cover and two books, one with a sidecar cover, series `T`, and the oneshots
/// `one` and `two`.
async fn series_library(directory: &Path) -> (TestServer, SqlitePool) {
    let server = TestServer::start_claimed(directory).await;
    let root = directory.join("comics");
    let book = cbz::zip(&[("1.png", STORED, &cover())]);
    for (path, content) in [
        ("S/s1.cbz", book.clone()),
        ("S/s1.png", cbz::fixture(PNG)),
        ("S/s2.cbz", book.clone()),
        ("S/cover.png", cbz::fixture(PNG)),
        ("T/t.cbz", book.clone()),
        ("_oneshots/one.cbz", book.clone()),
        ("_oneshots/two.cbz", book),
    ] {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("Folder")).expect("Folder");
        fs::write(path, content).expect("Library file");
    }
    let library = json!({"name": "Comics", "root": root, "oneshotsDirectory": "_oneshots"});
    let response = fetch(
        server
            .admin(Method::POST, "/api/v1/libraries")
            .json(&library),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    wait_for_scan_work(directory).await;
    let pool = db::main_pool(directory).await;
    (server, pool)
}

pub(crate) async fn delete(server: &TestServer, directory: &Path, book_id: &str) -> Vec<TaskRow> {
    delete_file(server, directory, &format!("/api/v1/books/{book_id}/file")).await
}

async fn delete_file(server: &TestServer, directory: &Path, path: &str) -> Vec<TaskRow> {
    let response = fetch(server.admin(Method::DELETE, path)).await;
    assert_eq!(response.status, StatusCode::ACCEPTED);
    let failed = |rows: &[TaskRow]| rows.iter().all(|row| row.3.contains("\"eraFailed\""));
    wait_for_tasks(directory, failed).await
}

/// `(TYPE, BOOK_ID, SERIES_ID, name)` of each historical event, by name.
async fn history(pool: &SqlitePool) -> Vec<(String, Option<String>, Option<String>, String)> {
    sqlx::query_as(
        "SELECT E.TYPE, E.BOOK_ID, E.SERIES_ID, P.VALUE FROM HISTORICAL_EVENT E \
         JOIN HISTORICAL_EVENT_PROPERTIES P ON P.ID = E.ID AND P.KEY = 'name' ORDER BY P.VALUE",
    )
    .fetch_all(pool)
    .await
    .expect("History")
}

fn deleted_ids(dates: &[(String, String)]) -> Vec<&str> {
    dates.iter().map(|(id, _)| id.as_str()).collect()
}

#[tokio::test]
async fn deleting_a_book_file_removes_its_sidecars_and_its_folder_once_empty() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = library(directory.path()).await;
    let root = directory.path().join("comics");
    let reader = create_reader(&server, "reader@example.com", json!({})).await;
    let (a, a_series) = book(&pool, "a").await;
    let denied = reader.request(&server, Method::DELETE, &format!("/api/v1/books/{a}/file"));
    assert_eq!(fetch(denied).await.status, StatusCode::FORBIDDEN);

    assert_eq!(delete(&server, directory.path(), &a).await, []);
    assert!(!root.join("A/a.cbz").exists());
    assert!(!root.join("A/a.png").exists(), "the sidecar cover goes too");
    assert!(root.join("A/b.cbz").exists());
    assert_eq!(deleted_ids(&deletion_dates(&pool).await), [a.as_str()]);
    let name = root.join("A").join("a.cbz").to_string_lossy().into_owned();
    let deleted = (
        "BookFileDeleted".to_owned(),
        Some(a.clone()),
        Some(a_series),
        name,
    );
    assert_eq!(history(&pool).await, std::slice::from_ref(&deleted));
    // Replayed, the deletion of a deleted book records nothing.
    delete(&server, directory.path(), &a).await;
    assert_eq!(history(&pool).await, [deleted]);

    // Emptied folders go, the library root stays even when emptied too.
    for name in ["b", "c", "d"] {
        let (id, _) = book(&pool, name).await;
        assert_eq!(delete(&server, directory.path(), &id).await, [], "{name}");
    }
    assert!(!root.join("A").exists() && !root.join("C").exists());
    assert_eq!(fs::read_dir(&root).expect("Root").count(), 0);
    let folders: Vec<_> = history(&pool)
        .await
        .into_iter()
        .filter(|event| event.0 == "SeriesFolderDeleted")
        .map(|event| (event.1, event.3))
        .collect();
    let folder = |name: &str| (None, root.join(name).to_string_lossy().into_owned());
    assert_eq!(folders, [folder("A"), folder("C")]);
    server.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn missing_or_read_only_book_files_are_left_alone() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = library(directory.path()).await;
    let root = directory.path().join("comics");
    let ((b, _), (c, _)) = (book(&pool, "b").await, book(&pool, "c").await);
    fs::remove_file(root.join("A/b.cbz")).expect("Remove b");
    let c_path = root.join("C/c.cbz");
    fs::set_permissions(&c_path, fs::Permissions::from_mode(0o444)).expect("Read-only c");

    delete(&server, directory.path(), &b).await;
    delete(&server, directory.path(), &c).await;

    assert!(c_path.exists());
    assert_eq!(deletion_dates(&pool).await, []);
    assert_eq!(history(&pool).await, []);
    server.stop().await;
}

#[tokio::test]
async fn deleting_series_files_removes_its_books_sidecars_and_folder() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = series_library(directory.path()).await;
    let root = directory.path().join("comics");
    let ((s1, s), (s2, _)) = (book(&pool, "s1").await, book(&pool, "s2").await);
    let path = format!("/api/v1/series/{s}/file");
    let reader = create_reader(&server, "reader@example.com", json!({})).await;
    let denied = reader.request(&server, Method::DELETE, &path);
    assert_eq!(fetch(denied).await.status, StatusCode::FORBIDDEN);

    assert_eq!(delete_file(&server, directory.path(), &path).await, []);
    let series = root.join("S");
    assert!(!series.exists(), "books, sidecars and folder go");
    assert!(root.join("T/t.cbz").exists());
    let mut deleted = vec![s1.clone(), s2.clone(), s.clone()];
    deleted.sort();
    assert_eq!(deleted_ids(&deletion_dates(&pool).await), deleted);
    let file = |id: &str, name: &str| {
        (
            "BookFileDeleted".to_owned(),
            Some(id.to_owned()),
            Some(s.clone()),
            series.join(name).to_string_lossy().into_owned(),
        )
    };
    let folder = (
        "SeriesFolderDeleted".to_owned(),
        None,
        Some(s.clone()),
        series.to_string_lossy().into_owned(),
    );
    let events = [folder, file(&s1, "s1.cbz"), file(&s2, "s2.cbz")];
    assert_eq!(history(&pool).await, events);

    // A oneshot book goes with its series, leaving the other oneshots.
    let (one, one_series) = book(&pool, "one").await;
    delete(&server, directory.path(), &one).await;
    assert!(!root.join("_oneshots/one.cbz").exists());
    assert!(root.join("_oneshots/two.cbz").exists());
    let deleted = deletion_dates(&pool).await;
    assert!(deleted_ids(&deleted).contains(&one.as_str()));
    assert!(deleted_ids(&deleted).contains(&one_series.as_str()));
    assert_eq!(history(&pool).await.len(), 4);
    server.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn missing_or_read_only_series_folders_are_left_alone() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = series_library(directory.path()).await;
    let folder = directory.path().join("comics/T");
    let ((_, s), (_, t)) = (book(&pool, "s1").await, book(&pool, "t").await);
    fs::remove_dir_all(directory.path().join("comics/S")).expect("Remove S");
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).expect("Read-only T");

    for series in [s, t] {
        let path = format!("/api/v1/series/{series}/file");
        delete_file(&server, directory.path(), &path).await;
    }

    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).expect("Writable T");
    assert!(folder.join("t.cbz").exists());
    assert_eq!(deletion_dates(&pool).await, []);
    assert_eq!(history(&pool).await, []);
    server.stop().await;
}
