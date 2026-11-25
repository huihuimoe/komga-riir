//! Library scans over real HTTP against real directories.

use std::fs::{self, File};
use std::path::Path;
use std::time::{Duration, SystemTime};

use komga_types::FileUrl;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::{admin_json, create, route};
use crate::support::db;
use crate::support::db::content::{
    self, ScannedBook, book_hashes, book_series, media, scanned_books, series_with_urls,
};
use crate::support::tasks::{
    TaskRow, clear_after_scans, tasks, wait_for_scan_work, wait_for_tasks,
};
use crate::support::{TestServer, openapi};

mod changed;
mod missing;
mod restored;
mod sidecars;

const READER: (&str, &str) = ("reader@example.com", "reader-password");
/// Kotlin `Hasher` and `KoreaderHasher` of a file holding `book bytes`.
const BOOK_BYTES_HASH: &str = "1ccb0906d89039e26aa6b2389261935d";
const BOOK_BYTES_KOREADER_HASH: &str = "bcf03edc190d3f59347767684ba25f76";

pub(super) async fn scan(server: &TestServer, library: &Value, query: &str) {
    let response = server
        .admin(Method::POST, &format!("{}/scan{query}", route(library)))
        .send()
        .await
        .expect("Scan request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    openapi::checked(Method::POST, response).await;
}

pub(super) fn touch(path: &Path) {
    fs::create_dir_all(path.parent().expect("Book directory")).expect("Book directory");
    fs::write(path, b"book bytes").expect("Book file");
}

/// Windows opens a directory only with backup semantics, and sets its times only through a
/// handle allowed to write attributes.
pub(super) fn set_directory_modified(directory: &Path, time: SystemTime) {
    let mut options = File::options();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_WRITE_ATTRIBUTES,
        };
        options
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    }
    options
        .open(directory)
        .and_then(|directory| directory.set_modified(time))
        .expect("Directory time");
}

#[tokio::test]
async fn scan_requires_an_admin_an_existing_library_and_a_boolean_deep() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let root = directory.path().join("comics");
    fs::create_dir(&root).expect("Library root");
    let library = create(&server, "Comics", &root).await;
    let ids: Vec<String> = wait_for_scan_work(directory.path())
        .await
        .into_iter()
        .map(|row| row.0)
        .collect();
    assert_eq!(
        ids,
        Vec::<String>::new(),
        "the library-wide follow-ups of an empty library ran"
    );
    clear_after_scans(&server, directory.path()).await;
    admin_json(
        &server,
        Method::POST,
        "/api/v2/users",
        Some(&json!({"email": READER.0, "password": READER.1})),
        StatusCode::CREATED,
    )
    .await;
    let path = format!("{}/scan", route(&library));
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
        // Spring binds `deep` before checking the role.
        (
            server
                .request(Method::POST, &format!("{path}?deep=maybe"))
                .basic_auth(READER.0, Some(READER.1)),
            StatusCode::BAD_REQUEST,
        ),
        (
            server.admin(Method::POST, "/api/v1/libraries/missing/scan"),
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

/// A server whose library `comics` holds `Series A` (three books) and a root-level book, after
/// the creation scan.
async fn scanned_library(directory: &Path) -> (TestServer, Value, SqlitePool, Vec<TaskRow>) {
    let server = TestServer::start_claimed(directory).await;
    let root = directory.join("comics");
    for book in [
        "Series A/a 10.cbz",
        "Series A/a 2.cbz",
        "Series A/a 1.cbz",
        "one.pdf",
    ] {
        touch(&root.join(book));
    }
    let library = create(&server, "Comics", &root).await;
    let pool = db::main_pool(directory).await;
    let tasks = wait_for_scan_work(directory).await;
    (server, library, pool, tasks)
}

#[tokio::test]
async fn a_scan_stores_series_and_numbered_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool, _) = scanned_library(directory.path()).await;

    let base = FileUrl::file(&directory.path().join("comics")).to_string();
    let found = series_with_urls(&pool).await;
    let names: Vec<(&str, &str, i32, bool)> = found
        .iter()
        .map(|row| (row.1.as_str(), row.2.as_str(), row.3, row.4))
        .collect();
    assert_eq!(
        names,
        [
            ("Series A", format!("{base}/Series%20A/").as_str(), 3, false),
            ("comics", format!("{base}/").as_str(), 1, false),
        ]
    );
    let stored = scanned_books(&pool).await;
    let rows: Vec<(&str, String, i32, i64, &str, f64, &str)> = stored
        .iter()
        .map(|row| {
            let url = row.2.strip_prefix(&base).expect("Book under the root");
            (
                row.1.as_str(),
                url.to_owned(),
                row.3,
                row.4,
                row.5.as_str(),
                row.6,
                row.7.as_str(),
            )
        })
        .collect();
    // New books are analyzed after the scan; their bytes are no book format.
    let expected = [
        (
            "a 1",
            "/Series%20A/a%201.cbz",
            1,
            10,
            "1",
            1.0,
            "UNSUPPORTED",
        ),
        (
            "a 2",
            "/Series%20A/a%202.cbz",
            2,
            10,
            "2",
            2.0,
            "UNSUPPORTED",
        ),
        (
            "a 10",
            "/Series%20A/a%2010.cbz",
            3,
            10,
            "3",
            3.0,
            "UNSUPPORTED",
        ),
        ("one", "/one.pdf", 1, 10, "1", 1.0, "UNSUPPORTED"),
    ]
    .map(|(name, url, number, size, metadata, sort, status)| {
        (name, url.to_owned(), number, size, metadata, sort, status)
    });
    assert_eq!(rows, expected);
    // Each series metadata refresh ran, then the aggregation of its book metadata.
    let aggregated = content::aggregated_since(&pool, &[]).await;
    assert_eq!(aggregated.len(), 2, "{aggregated:?}");
    server.stop().await;
}

#[tokio::test]
async fn rescans_keep_identities_and_add_new_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool, _) = scanned_library(directory.path()).await;
    let found = series_with_urls(&pool).await;
    let stored = scanned_books(&pool).await;

    // An unchanged tree keeps every identity, deep or not.
    for query in ["", "?deep=true"] {
        scan(&server, &library, query).await;
        wait_for_scan_work(directory.path()).await;
        assert_eq!(series_with_urls(&pool).await, found);
        assert_eq!(scanned_books(&pool).await, stored);
    }

    // A new file changes the directory; Kotlin compares times at millisecond precision.
    let series_directory = directory.path().join("comics/Series A");
    touch(&series_directory.join("a 3.cbz"));
    set_directory_modified(
        &series_directory,
        SystemTime::now() + Duration::from_secs(60),
    );
    scan(&server, &library, "").await;
    wait_for_scan_work(directory.path()).await;
    let after = scanned_books(&pool).await;
    let id = |rows: &[ScannedBook], name: &str| -> String {
        rows.iter()
            .find(|row| row.1 == name)
            .map(|row| row.0.clone())
            .expect("Stored book")
    };
    let series_a: Vec<(String, i32)> = after
        .iter()
        .filter(|row| row.2.contains("Series%20A"))
        .map(|row| (row.0.clone(), row.3))
        .collect();
    assert_eq!(
        series_a,
        [
            (id(&stored, "a 1"), 1),
            (id(&stored, "a 2"), 2),
            (id(&after, "a 3"), 3),
            (id(&stored, "a 10"), 4),
        ]
    );
    assert_eq!(series_with_urls(&pool).await[0].3, 4);
    server.stop().await;
}

#[tokio::test]
async fn an_inaccessible_or_empty_root_marks_the_library_unavailable_and_keeps_its_content() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let root = directory.path().join("comics");
    touch(&root.join("Series/a.cbz"));
    let library = create(&server, "Comics", &root).await;
    let pool = db::main_pool(directory.path()).await;
    wait_for_scan_work(directory.path()).await;
    let stored = scanned_books(&pool).await;
    assert_eq!(stored.len(), 1);

    let moved = directory.path().join("moved");
    fs::rename(&root, &moved).expect("Move root away");
    scan(&server, &library, "").await;
    let tasks = wait_for_scan_work(directory.path()).await;
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
    assert_eq!(
        scanned_books(&pool).await,
        stored,
        "an unreadable root never empties the library"
    );

    // An empty root, like the mount point of an unmounted share, empties nothing either.
    fs::create_dir(&root).expect("Empty root");
    scan(&server, &library, "").await;
    let tasks = wait_for_scan_work(directory.path()).await;
    let scan_task = tasks
        .iter()
        .find(|row| row.0.starts_with("SCAN_LIBRARY_"))
        .expect("Failed scan");
    assert!(scan_task.3.contains("Folder has no books"), "{scan_task:?}");
    let details = admin_json(&server, Method::GET, &route(&library), None, StatusCode::OK).await;
    assert_eq!(details["unavailable"], json!(true));
    assert_eq!(scanned_books(&pool).await, stored);

    fs::remove_dir(&root).expect("Remove empty root");
    fs::rename(&moved, &root).expect("Restore root");
    scan(&server, &library, "").await;
    wait_for_scan_work(directory.path()).await;
    let details = admin_json(&server, Method::GET, &route(&library), None, StatusCode::OK).await;
    assert_eq!(details["unavailable"], json!(false));
    assert_eq!(scanned_books(&pool).await, stored);
    server.stop().await;
}

#[tokio::test]
async fn a_scan_runs_its_follow_ups() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool, tasks) = scanned_library(directory.path()).await;

    // New books have UNKNOWN media and no hash; only `hashFiles` is on by default. Without a media
    // type no extension can mismatch. The queued analyses, file hashes, and the searches for pages
    // to hash, books to convert and duplicate pages to delete ran: every book has its media type and the hash Kotlin computes
    // for `book bytes`.
    for (book, _) in book_series(&pool).await {
        assert_eq!(media(&pool, &book).await.0, "UNSUPPORTED", "{book}");
        assert_eq!(
            book_hashes(&pool, &book).await,
            (BOOK_BYTES_HASH.to_owned(), String::new()),
            "{book}"
        );
    }
    let follow_ups: Vec<_> = tasks
        .iter()
        .filter(|row| !row.0.starts_with("REFRESH_"))
        .collect();
    assert!(follow_ups.is_empty(), "{follow_ups:?}");
    server.stop().await;
}

#[tokio::test]
async fn enabling_library_options_queues_their_follow_ups() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool, _) = scanned_library(directory.path()).await;
    clear_after_scans(&server, directory.path()).await;
    let books = book_series(&pool).await;
    let update = |body: Value| {
        let path = route(&library);
        let server = &server;
        async move {
            admin_json(
                server,
                Method::PATCH,
                &path,
                Some(&body),
                StatusCode::NO_CONTENT,
            )
            .await;
        }
    };

    // Turning hashing off and the others on: no rescan, only the switched-on options queue.
    update(json!({
        "hashFiles": false,
        "hashKoreader": true,
        "hashPages": true,
        "repairExtensions": true,
        "convertToCbz": true,
    }))
    .await;
    // The queued KOReader hashes, the searches for pages to hash and books to convert run.
    wait_for_tasks(directory.path(), <[TaskRow]>::is_empty).await;
    for (book, _) in &books {
        assert_eq!(
            book_hashes(&pool, book).await,
            (
                BOOK_BYTES_HASH.to_owned(),
                BOOK_BYTES_KOREADER_HASH.to_owned()
            ),
            "{book}"
        );
    }

    clear_after_scans(&server, directory.path()).await;
    update(json!({"hashKoreader": true, "hashPages": true, "convertToCbz": true})).await;
    assert!(
        tasks(directory.path()).await.is_empty(),
        "options already on queue nothing"
    );
    update(json!({"hashFiles": true})).await;
    assert!(
        tasks(directory.path()).await.is_empty(),
        "books hashed by the creation scan queue no file hash"
    );
    server.stop().await;
}
