//! Task recovery after the server process is killed.

use std::fs;
use std::time::Duration;

use reqwest::header::COOKIE;
use reqwest::{Client, StatusCode};
use serde_json::json;
use tokio::time;

use crate::support::cbz;
use crate::support::db::{self, WriteLock};
use crate::support::process::{Server, admin_session, create_library, free_port};
use crate::support::tasks::{wait_for_scan_work, wait_for_tasks};
use crate::support::{ADMIN_EMAIL, ADMIN_PASSWORD};

async fn empty_trash(client: &Client, server: &Server, cookie: &str, library: &str) {
    let response = client
        .post(format!(
            "{}/api/v1/libraries/{library}/empty-trash",
            server.base_url
        ))
        .header(COOKIE, cookie)
        .send()
        .await
        .expect("Empty trash");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn killed_running_and_unstarted_tasks_each_run_once_after_restart() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let port = free_port();
    let client = Client::new();
    let server = Server::start(directory.path(), port, "first.log").await;
    let cookie = admin_session(&client, &server).await;
    let running = create_library(&client, &server, &directory.path().join("running")).await;
    let queued = create_library(&client, &server, &directory.path().join("queued")).await;
    // Creating a library queues its first scan, which leaves follow-ups this build does not run.
    wait_for_scan_work(directory.path()).await;
    let response = client
        .delete(format!("{}/api/v1/tasks", server.base_url))
        .header(COOKIE, &cookie)
        .send()
        .await
        .expect("Clear task queue");
    assert_eq!(response.status(), StatusCode::OK);

    // The main database write lock holds the worker inside the first task.
    let lock = WriteLock::acquire(directory.path()).await;
    empty_trash(&client, &server, &cookie, &running).await;
    wait_for_tasks(directory.path(), |rows| {
        rows.iter().any(|row| row.4.is_some())
    })
    .await;
    empty_trash(&client, &server, &cookie, &queued).await;
    server.kill();
    lock.release().await;
    let rows = wait_for_tasks(directory.path(), |rows| rows.len() == 2).await;
    let owners: Vec<(&str, bool)> = rows
        .iter()
        .map(|row| (row.0.as_str(), row.4.is_some()))
        .collect();
    let mut expected = [
        (format!("EMPTY_TRASH_{running}"), true),
        (format!("EMPTY_TRASH_{queued}"), false),
    ];
    expected.sort();
    let expected: Vec<(&str, bool)> = expected
        .iter()
        .map(|(id, owned)| (id.as_str(), *owned))
        .collect();
    assert_eq!(
        owners, expected,
        "the kill left one task running and one queued"
    );

    let server = Server::start(directory.path(), port, "second.log").await;
    wait_for_tasks(directory.path(), <[_]>::is_empty).await;
    let log = server.kill();
    assert!(
        log.contains("Reset tasks that were not finished released=1"),
        "{log}"
    );
    for library in [running, queued] {
        let executions = log
            .lines()
            .filter(|line| line.contains("Task executed") && line.contains(&library))
            .count();
        assert_eq!(executions, 1, "{library} after restart:\n{log}");
    }
}

#[tokio::test]
async fn tasks_committed_with_a_change_are_queued_after_a_kill_before_their_hand_over() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let port = free_port();
    let client = Client::new();
    let server = Server::start(directory.path(), port, "first.log").await;
    admin_session(&client, &server).await;

    // The tasks database write lock holds the hand-over of the library's first scan.
    let lock = WriteLock::acquire_tasks(directory.path()).await;
    let root = directory.path().join("library");
    let creation = tokio::spawn({
        let (client, base_url) = (client.clone(), server.base_url.clone());
        async move {
            fs::create_dir(&root).expect("Library root");
            client
                .post(format!("{base_url}/api/v1/libraries"))
                .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
                .json(&json!({"name": "library", "root": root}))
                .send()
                .await
        }
    });
    let main = db::main_pool(directory.path()).await;
    let staged = time::timeout(Duration::from_secs(10), async {
        loop {
            let ids: Vec<String> = sqlx::query_scalar("SELECT ID FROM ERA_TASK_HANDOVER")
                .fetch_all(&main)
                .await
                .expect("Staged tasks");
            if !ids.is_empty() {
                return ids;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("The library is committed with its scan staged");
    server.kill();
    creation.abort();
    lock.release().await;
    assert!(wait_for_tasks(directory.path(), |_| true).await.is_empty());

    let server = Server::start(directory.path(), port, "second.log").await;
    wait_for_scan_work(directory.path()).await;
    let log = server.kill();
    let library: String = sqlx::query_scalar("SELECT ID FROM LIBRARY")
        .fetch_one(&main)
        .await
        .expect("Committed library");
    assert_eq!(staged, [format!("SCAN_LIBRARY_{library}_DEEP_false")]);
    assert!(
        log.lines()
            .any(|line| line.contains("Task executed") && line.contains(&library)),
        "{log}"
    );
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ERA_TASK_HANDOVER")
        .fetch_one(&main)
        .await
        .expect("Staged tasks");
    assert_eq!(left, 0);
}

#[tokio::test]
async fn a_scan_killed_after_its_commit_but_before_it_ends_is_replayed_without_duplicates() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let port = free_port();
    let client = Client::new();
    let server = Server::start(directory.path(), port, "first.log").await;
    let cookie = admin_session(&client, &server).await;
    let root = directory.path().join("library");
    let library = create_library(&client, &server, &root).await;
    wait_for_scan_work(directory.path()).await;
    fs::create_dir(root.join("Series")).expect("Series directory");
    let book = cbz::zip(&[("1.png", cbz::STORED, b"png")]);
    fs::write(root.join("Series/book.cbz"), book).expect("Book file");

    // The main database lock holds the scan until the tasks database is locked too; then the
    // scan commits its books and waits to queue their follow-ups and to end.
    let main_lock = WriteLock::acquire(directory.path()).await;
    let response = client
        .post(format!(
            "{}/api/v1/libraries/{library}/scan",
            server.base_url
        ))
        .header(COOKIE, &cookie)
        .send()
        .await
        .expect("Scan");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    wait_for_tasks(directory.path(), |rows| {
        rows.iter().any(|row| row.4.is_some())
    })
    .await;
    let tasks_lock = WriteLock::acquire_tasks(directory.path()).await;
    main_lock.release().await;
    let main = db::main_pool(directory.path()).await;
    db::wait_for_rows(&main, "SELECT COUNT(*) FROM ERA_TASK_HANDOVER").await;
    server.kill();
    tasks_lock.release().await;
    let rows = wait_for_tasks(directory.path(), |_| true).await;
    assert!(
        rows.iter()
            .any(|row| row.0.starts_with("SCAN_LIBRARY_") && row.4.is_some()),
        "the scan is still running: {rows:?}"
    );

    let server = Server::start(directory.path(), port, "second.log").await;
    wait_for_scan_work(directory.path()).await;
    let log = server.kill();
    assert!(
        log.contains("Reset tasks that were not finished released=1"),
        "{log}"
    );
    for (query, expected) in [
        ("SELECT COUNT(*) FROM BOOK", 1),
        ("SELECT COUNT(*) FROM SERIES", 1),
        ("SELECT COUNT(*) FROM ERA_TASK_HANDOVER", 0),
    ] {
        let count: i64 = sqlx::query_scalar(query)
            .fetch_one(&main)
            .await
            .expect("Count rows");
        assert_eq!(count, expected, "{query}");
    }
    let analyses = log
        .lines()
        .filter(|line| line.contains("Task executed") && line.contains("AnalyzeBook"))
        .count();
    assert_eq!(analyses, 1, "{log}");
}
