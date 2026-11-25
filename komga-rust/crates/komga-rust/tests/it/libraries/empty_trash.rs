use std::fs;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::scan::{scan, touch};
use super::{admin_json, create, route};
use crate::support::db::content::{self, numbered_books, series};
use crate::support::db::{self, WriteLock};
use crate::support::tasks::{TaskRow, clear_after_scans, tasks, wait_for_tasks};
use crate::support::{TestServer, openapi};

const READER: (&str, &str) = ("reader@example.com", "reader-password");

async fn create_libraries(server: &TestServer, directory: &Path) -> [serde_json::Value; 2] {
    let mut libraries = Vec::new();
    for name in ["first", "second"] {
        let root = directory.join(name);
        std::fs::create_dir(&root).expect("Library root");
        libraries.push(create(server, name, &root).await);
    }
    // Creating a library queues its first scan.
    clear_after_scans(server, directory).await;
    <[_; 2]>::try_from(libraries).expect("Two libraries")
}

/// The admin session authenticates without writing to the locked main database.
async fn empty_trash(server: &TestServer, path: &str) -> reqwest::Response {
    server
        .admin(Method::POST, &format!("{path}/empty-trash"))
        .send()
        .await
        .expect("Empty trash request")
}

#[tokio::test]
async fn empty_trash_requires_an_admin_and_an_existing_library() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let root = directory.path().join("comics");
    std::fs::create_dir(&root).expect("Library root");
    let library = create(&server, "Comics", &root).await;
    clear_after_scans(&server, directory.path()).await;
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&json!({"email": READER.0, "password": READER.1}))
        .send()
        .await
        .expect("Create reader");
    assert_eq!(response.status(), StatusCode::CREATED);
    let path = format!("{}/empty-trash", route(&library));
    for (request, status) in [
        (
            server.request(Method::POST, &path),
            StatusCode::UNAUTHORIZED,
        ),
        (
            server
                .request(Method::POST, &path)
                .basic_auth(READER.0, Some(READER.1)),
            StatusCode::FORBIDDEN,
        ),
        (
            server.admin(Method::POST, "/api/v1/libraries/missing/empty-trash"),
            StatusCode::NOT_FOUND,
        ),
    ] {
        assert_eq!(request.send().await.expect("Request").status(), status);
    }
    assert!(
        tasks(directory.path()).await.is_empty(),
        "rejected requests queue nothing"
    );
    server.stop().await;
}

#[tokio::test]
async fn empty_trash_is_persisted_deduplicated_and_executed_by_a_worker() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let [first, second] = create_libraries(&server, directory.path()).await;

    let lock = WriteLock::acquire(directory.path()).await;
    let response = empty_trash(&server, &route(&first)).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    openapi::checked(Method::POST, response).await;
    let running = wait_for_tasks(directory.path(), |rows| {
        rows.iter().any(|row| row.4.is_some())
    })
    .await;
    let first_id = first["id"].as_str().expect("Library ID");
    let [(id, priority, class, payload, owner)] = &running[..] else {
        panic!("Expected one running task, got {running:?}");
    };
    assert_eq!(
        (id.as_str(), *priority, class.as_str(), owner.as_deref()),
        (
            format!("EMPTY_TRASH_{first_id}").as_str(),
            6,
            "org.gotson.komga.application.tasks.Task$EmptyTrash",
            Some("taskProcessor-1"),
        )
    );
    // Kotlin's Jackson mapper ignores the retry fields.
    let payload: serde_json::Value = serde_json::from_str(payload).expect("JSON payload");
    assert_eq!(
        payload,
        json!({"libraryId": first_id, "priority": 6, "eraAttempts": 1})
    );

    for _ in 0..2 {
        let response = empty_trash(&server, &route(&second)).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }
    let queued = tasks(directory.path()).await;
    assert_eq!(
        queued.len(),
        2,
        "the second library is queued once: {queued:?}"
    );
    let second_id = second["id"].as_str().expect("Library ID");
    assert!(
        queued
            .iter()
            .any(|row| row.0 == format!("EMPTY_TRASH_{second_id}") && row.4.is_none()),
        "{queued:?}"
    );

    lock.release().await;
    wait_for_tasks(directory.path(), <[TaskRow]>::is_empty).await;
    server.stop().await;
}

#[tokio::test]
async fn a_task_pool_size_update_applies_without_a_restart() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let libraries = create_libraries(&server, directory.path()).await;
    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({"taskPoolSize": 2}))
        .send()
        .await
        .expect("Settings update");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let lock = WriteLock::acquire(directory.path()).await;
    for library in &libraries {
        let response = empty_trash(&server, &route(library)).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }
    let running = wait_for_tasks(directory.path(), |rows| {
        rows.len() == 2 && rows.iter().all(|row| row.4.is_some())
    })
    .await;
    let mut owners: Vec<&str> = running.iter().filter_map(|row| row.4.as_deref()).collect();
    owners.sort_unstable();
    assert_eq!(owners, ["taskProcessor-1", "taskProcessor-2"]);

    lock.release().await;
    wait_for_tasks(directory.path(), <[TaskRow]>::is_empty).await;
    server.stop().await;
}

#[tokio::test]
async fn clearing_the_task_queue_cancels_only_unclaimed_tasks() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let [first, second] = create_libraries(&server, directory.path()).await;
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&json!({"email": READER.0, "password": READER.1}))
        .send()
        .await
        .expect("Create reader");
    assert_eq!(response.status(), StatusCode::CREATED);
    for (request, status) in [
        (
            server.request(Method::DELETE, "/api/v1/tasks"),
            StatusCode::UNAUTHORIZED,
        ),
        (
            server
                .request(Method::DELETE, "/api/v1/tasks")
                .basic_auth(READER.0, Some(READER.1)),
            StatusCode::FORBIDDEN,
        ),
    ] {
        assert_eq!(request.send().await.expect("Request").status(), status);
    }

    let lock = WriteLock::acquire(directory.path()).await;
    empty_trash(&server, &route(&first)).await;
    wait_for_tasks(directory.path(), |rows| {
        rows.iter().any(|row| row.4.is_some())
    })
    .await;
    empty_trash(&server, &route(&second)).await;
    for expected in [1, 0] {
        let response = server
            .admin(Method::DELETE, "/api/v1/tasks")
            .send()
            .await
            .expect("Clear task queue");
        assert_eq!(openapi::checked(Method::DELETE, response).await, expected);
    }
    let first_id = first["id"].as_str().expect("Library ID");
    let remaining: Vec<(String, bool)> = tasks(directory.path())
        .await
        .into_iter()
        .map(|row| (row.0, row.4.is_some()))
        .collect();
    assert_eq!(remaining, [(format!("EMPTY_TRASH_{first_id}"), true)]);

    lock.release().await;
    wait_for_tasks(directory.path(), <[TaskRow]>::is_empty).await;
    server.stop().await;
}

async fn scan_all(server: &TestServer, directory: &Path, libraries: &[&Value]) {
    for library in libraries {
        scan(server, library, "").await;
    }
    clear_after_scans(server, directory).await;
}

#[tokio::test]
async fn empty_trash_deletes_only_trashed_series_and_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let [first, second] = create_libraries(&server, directory.path()).await;
    let path = |path: &str| directory.path().join(path);
    for book in [
        "first/Series A/a 1.cbz",
        "first/Series A/a 2.cbz",
        "first/Series A/a 10.cbz",
        "first/Series B/b.cbz",
        "second/Series C/c.cbz",
        // Keeps the other library from scanning empty, which deletes nothing.
        "second/Series D/d.cbz",
    ] {
        touch(&path(book));
    }
    scan_all(&server, directory.path(), &[&first, &second]).await;
    fs::remove_file(path("first/Series A/a 2.cbz")).expect("Remove book");
    fs::remove_dir_all(path("first/Series B")).expect("Remove series");
    fs::remove_file(path("second/Series C/c.cbz")).expect("Remove other library book");
    scan_all(&server, directory.path(), &[&first, &second]).await;
    let pool = db::main_pool(directory.path()).await;
    let stored = (series(&pool).await, numbered_books(&pool).await);
    let deleted: Vec<bool> = stored.1.iter().map(|row| row.4).collect();
    assert_eq!(
        deleted,
        [false, false, true, true, true, false],
        "{stored:?}"
    );
    let aggregations = content::aggregation_dates(&pool).await;

    admin_json(
        &server,
        Method::POST,
        &format!("{}/empty-trash", route(&first)),
        None,
        StatusCode::ACCEPTED,
    )
    .await;
    // Each refresh queues the next before its row goes, so the queue empties once all ran.
    wait_for_tasks(directory.path(), <[TaskRow]>::is_empty).await;
    // The surviving series is renumbered with the trashed book gone.
    let [a1, a10, _, _, c, d] = &stored.1[..] else {
        panic!("Six books: {stored:?}");
    };
    assert_eq!(
        numbered_books(&pool).await,
        [
            (a1.0.clone(), "a 1".to_owned(), 1, "1".to_owned(), false),
            (a10.0.clone(), "a 10".to_owned(), 2, "2".to_owned(), false),
            c.clone(),
            d.clone(),
        ]
    );
    let [series_a, _, series_c, series_d] = &stored.0[..] else {
        panic!("Four series: {stored:?}");
    };
    assert_eq!(
        series(&pool).await,
        [
            (series_a.0.clone(), "Series A".to_owned(), 2, false),
            series_c.clone(),
            series_d.clone(),
        ],
        "the other library keeps its trash"
    );
    // The renumbered book's metadata refresh ran, then the refresh of its series, then the
    // aggregation of its book metadata.
    assert_eq!(
        content::aggregated_since(&pool, &aggregations).await,
        [series_a.0.as_str()]
    );
    assert!(path("first/Series A/a 10.cbz").exists());

    // A scan of a library set to empty its trash afterwards does the same.
    admin_json(
        &server,
        Method::PATCH,
        &route(&second),
        Some(&json!({"emptyTrashAfterScan": true})),
        StatusCode::NO_CONTENT,
    )
    .await;
    scan_all(&server, directory.path(), &[&second]).await;
    assert_eq!(
        series(&pool).await,
        [
            (series_a.0.clone(), "Series A".to_owned(), 2, false),
            series_d.clone()
        ]
    );
    assert_eq!(numbered_books(&pool).await.len(), 3);
    server.stop().await;
}
