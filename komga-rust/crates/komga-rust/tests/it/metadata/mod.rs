//! Metadata refreshes over real HTTP against real book files.

mod aggregation;
mod barcode;
mod book_edit;
mod books;
mod collections;
mod epub;
mod mylar;
mod oneshot;
mod read_lists;
mod refresh;
mod search_index;
mod series;
mod series_edit;

use std::fs;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::libraries::{admin_json, route};
use crate::support::cbz::{self, STORED};
use crate::support::tasks::{TaskRow, wait_for_scan_work};
use crate::support::{TestServer, db};

const PNG: &str = "komga-image/tests/fixtures/photo.png";

/// A book of one PNG page and, unless `comic_info` is `None`, a `ComicInfo.xml` holding it.
fn comic(comic_info: Option<&str>) -> Vec<u8> {
    let png = cbz::fixture(PNG);
    let mut members: Vec<(&str, u16, &[u8])> = vec![("page.png", STORED, &png)];
    if let Some(xml) = comic_info {
        members.push(("ComicInfo.xml", STORED, xml.as_bytes()));
    }
    cbz::zip(&members)
}

fn write(directory: &Path, path: &str, content: &[u8]) {
    let path = directory.join("comics").join(path);
    fs::create_dir_all(path.parent().expect("Book directory")).expect("Book directory");
    fs::write(path, content).expect("Book file");
}

/// A claimed server whose library `comics` was scanned, analyzed, and refreshed.
async fn library(directory: &Path) -> (TestServer, Value, SqlitePool) {
    let server = TestServer::start_claimed(directory).await;
    let body = json!({"name": "Comics", "root": directory.join("comics")});
    let library = admin_json(
        &server,
        Method::POST,
        "/api/v1/libraries",
        Some(&body),
        StatusCode::OK,
    )
    .await;
    wait_for_scan_work(directory).await;
    let pool = db::main_pool(directory).await;
    (server, library, pool)
}

async fn rescan(server: &TestServer, directory: &Path, library: &Value) {
    scan(server, directory, library, "").await;
}

/// A deep scan, which refreshes the metadata of every series.
async fn rescan_deep(server: &TestServer, directory: &Path, library: &Value) {
    scan(server, directory, library, "?deep=true").await;
}

async fn scan(server: &TestServer, directory: &Path, library: &Value, query: &str) {
    let response = server
        .admin(Method::POST, &format!("{}/scan{query}", route(library)))
        .send()
        .await
        .expect("Scan request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_no_failed_task(wait_for_scan_work(directory).await);
}

fn assert_no_failed_task(rows: Vec<TaskRow>) {
    let failed: Vec<String> = rows
        .into_iter()
        .filter(|row| row.3.contains("\"eraFailed\""))
        .map(|row| row.2)
        .collect();
    assert_eq!(failed, Vec::<String>::new(), "no task fails");
}
