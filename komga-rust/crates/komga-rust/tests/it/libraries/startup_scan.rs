//! Libraries scanned when the server starts.

use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::time;

use super::scan::touch;
use super::{admin_json, create, route};
use crate::support::TestServer;
use crate::support::tasks::{clear_after_scans, wait_for_scan_work};

#[tokio::test]
async fn only_libraries_scanning_on_startup_find_books_added_while_stopped() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let mut libraries = Vec::new();
    for name in ["on-startup", "on-request"] {
        let root = directory.path().join(name);
        std::fs::create_dir(&root).expect("Library root");
        libraries.push(create(&server, name, &root).await);
    }
    admin_json(
        &server,
        Method::PATCH,
        &route(&libraries[0]),
        Some(&json!({"scanOnStartup": true})),
        StatusCode::NO_CONTENT,
    )
    .await;
    clear_after_scans(&server, directory.path()).await;
    server.stop().await;

    for name in ["on-startup", "on-request"] {
        touch(&directory.path().join(name).join("series/book.cbz"));
    }
    let server = TestServer::start(directory.path()).await;
    // A scan of the other library would have been queued with this one.
    time::timeout(Duration::from_secs(10), async {
        while book_count(&server, &libraries[0]).await == 0 {
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("The startup scan finds the book");
    wait_for_scan_work(directory.path()).await;
    assert_eq!(book_count(&server, &libraries[1]).await, 0);
    server.stop().await;
}

async fn book_count(server: &TestServer, library: &Value) -> u64 {
    let id = library["id"].as_str().expect("Library ID");
    let page = admin_json(
        server,
        Method::GET,
        &format!("/api/v1/books?library_id={id}"),
        None,
        StatusCode::OK,
    )
    .await;
    page["totalElements"].as_u64().expect("Book count")
}
