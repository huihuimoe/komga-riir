//! A library on a network share that disconnects. The test needs `KOMGA_TEST_NAS`, a directory on
//! a soft-mounted share it may write to, and `KOMGA_TEST_NAS_DISCONNECT`, a shell command that
//! stops the share's server, such as `pkill smbd` for a CIFS mount of a local Samba share:
//! `cargo nextest run --run-ignored only -- libraries::nas::`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::scan::scan;
use super::{admin_json, create, route};
use crate::support::cbz::{self, STORED};
use crate::support::db;
use crate::support::db::content::scanned_books;
use crate::support::posters::png;
use crate::support::tasks::{wait_for_scan_work, wait_for_scan_work_within};
use crate::support::{TestServer, json};

/// `KOMGA_TEST_NAS`/`komga-rust`, emptied.
fn library_root() -> PathBuf {
    let nas = std::env::var_os("KOMGA_TEST_NAS").expect("KOMGA_TEST_NAS");
    let root = PathBuf::from(nas).join("komga-rust");
    if root.exists() {
        fs::remove_dir_all(&root).expect("Empty the library root");
    }
    fs::create_dir_all(root.join("Series")).expect("Series directory");
    root
}

fn disconnect() {
    let command = std::env::var("KOMGA_TEST_NAS_DISCONNECT").expect("KOMGA_TEST_NAS_DISCONNECT");
    let status = Command::new("sh")
        .args(["-c", &command])
        .status()
        .expect("Disconnect the share");
    assert!(status.success(), "{command}: {status}");
}

async fn page(server: &TestServer, book: &str) -> StatusCode {
    server
        .admin(Method::GET, &format!("/api/v1/books/{book}/pages/1"))
        .send()
        .await
        .expect("Page request")
        .status()
}

async fn listing(server: &TestServer, path: &PathBuf) -> (StatusCode, Value) {
    let response = server
        .admin(Method::POST, "/api/v1/filesystem")
        .json(&json!({"path": path}))
        .send()
        .await
        .expect("Directory listing request");
    (response.status(), json(response).await)
}

#[tokio::test]
#[ignore = "needs KOMGA_TEST_NAS and KOMGA_TEST_NAS_DISCONNECT, a share it disconnects"]
async fn a_disconnected_share_fails_instead_of_looking_empty() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = library_root();
    let book = cbz::zip(&[("1.png", STORED, &png(10, 10))]);
    fs::write(root.join("Series/a.cbz"), book).expect("Book file");
    let server = TestServer::start_claimed(directory.path()).await;
    let library = create(&server, "Comics", &root).await;
    wait_for_scan_work(directory.path()).await;
    let pool = db::main_pool(directory.path()).await;
    let stored = scanned_books(&pool).await;
    let book: String = sqlx::query_scalar("SELECT ID FROM BOOK")
        .fetch_one(&pool)
        .await
        .expect("Book");
    assert_eq!(page(&server, &book).await, StatusCode::OK);

    disconnect();
    for path in [root.clone(), root.join("Series")] {
        let started = Instant::now();
        let (status, body) = listing(&server, &path).await;
        eprintln!(
            "listing {}: {status} in {:?}",
            path.display(),
            started.elapsed()
        );
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["message"], "Path does not exist");
    }
    let started = Instant::now();
    let status = page(&server, &book).await;
    eprintln!("page: {status} in {:?}", started.elapsed());
    assert!(
        status.is_client_error() || status.is_server_error(),
        "{status}"
    );

    let started = Instant::now();
    scan(&server, &library, "").await;
    // Each request to the disconnected share waits for the mount's own timeout.
    let tasks = wait_for_scan_work_within(directory.path(), Duration::from_secs(300)).await;
    eprintln!("scan in {:?}", started.elapsed());
    let scan_task = tasks
        .iter()
        .find(|row| row.0.starts_with("SCAN_LIBRARY_"))
        .expect("Failed scan");
    assert!(
        scan_task.3.contains("Folder is not accessible"),
        "{scan_task:?}"
    );
    let details = admin_json(&server, Method::GET, &route(&library), None, StatusCode::OK).await;
    assert_eq!(details["unavailable"], json!(true));
    assert_eq!(scanned_books(&pool).await, stored);
    server.stop().await;
}
