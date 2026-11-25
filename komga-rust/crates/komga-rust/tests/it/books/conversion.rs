//! RAR books converted to CBZ and book extensions repaired (`BookConverter`).

use std::fs;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use super::{JPEG, library_with_books};
use crate::libraries::{admin_json, route};
use crate::support::db::content::{book, media, pages};
use crate::support::tasks::{
    queue_kotlin_conversion, queue_kotlin_repair, wait_for_scan_work, wait_for_tasks,
};
use crate::support::{TestServer, cbz, db};

fn rar(name: &str) -> Vec<u8> {
    cbz::fixture(&format!("komga-archive/tests/fixtures/rar/{name}"))
}

async fn url(pool: &SqlitePool, book_id: &str) -> String {
    sqlx::query_scalar("SELECT URL FROM BOOK WHERE ID = ?")
        .bind(book_id)
        .fetch_one(pool)
        .await
        .expect("Book URL")
}

/// Enables the conversion and waits until every conversion task ended or failed.
async fn convert(server: &TestServer, directory: &Path, library: &serde_json::Value) {
    enable(server, directory, library, "convertToCbz").await;
}

/// Enables the library `option` and waits until every conversion task ended or failed.
async fn enable(server: &TestServer, directory: &Path, library: &serde_json::Value, option: &str) {
    let body = json!({option: true});
    admin_json(
        server,
        Method::PATCH,
        &route(library),
        Some(&body),
        StatusCode::NO_CONTENT,
    )
    .await;
    wait_for_conversions(directory).await;
}

async fn wait_for_conversions(directory: &Path) {
    wait_for_tasks(directory, |rows| {
        rows.iter()
            .filter(|row| {
                ["$ConvertBook", "$FindBooksToConvert", "$RepairExtension"]
                    .iter()
                    .any(|class| row.2.ends_with(class))
            })
            .all(|row| row.3.contains("\"eraFailed\""))
    })
    .await;
}

/// `(ID, SERIES_ID)` of the book whose file is named `name`.
async fn book_at(pool: &SqlitePool, name: &str) -> (String, String) {
    sqlx::query_as("SELECT ID, SERIES_ID FROM BOOK WHERE URL LIKE '%/' || ?")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("Book")
}

fn leftovers(directory: &Path) -> Vec<String> {
    let entries = fs::read_dir(directory).expect("Series directory");
    let names = entries.map(|entry| {
        entry
            .expect("Entry")
            .file_name()
            .to_string_lossy()
            .into_owned()
    });
    names.filter(|name| name.starts_with(".komga-")).collect()
}

#[tokio::test]
async fn rar_books_become_cbz_keeping_their_id_page_hashes_and_progress() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/four.cbr", rar("rar4-solid.rar")),
            ("Series/five.cbr", rar("rar5-solid.rar")),
        ],
    )
    .await;
    let (four, _) = book(&pool, "four").await;
    let (five, _) = book(&pool, "five").await;
    let before = pages(&pool, &four).await;
    sqlx::query("UPDATE MEDIA_PAGE SET FILE_HASH = 'kept' WHERE BOOK_ID = ? AND NUMBER = 0")
        .bind(&four)
        .execute(&pool)
        .await
        .expect("Page hash");
    let progress = format!("/api/v1/books/{four}/read-progress");
    let reading = server
        .admin(Method::PATCH, &progress)
        .json(&json!({"page": 2}));
    assert_eq!(
        reading.send().await.expect("Progress").status(),
        StatusCode::NO_CONTENT
    );

    convert(&server, directory.path(), &library).await;

    let series = directory.path().join("comics/Series");
    for (id, name) in [(&four, "four"), (&five, "five")] {
        assert!(
            url(&pool, id)
                .await
                .ends_with(&format!("Series/{name}.cbz")),
            "{name}"
        );
        assert!(series.join(format!("{name}.cbz")).is_file(), "{name}");
        assert!(!series.join(format!("{name}.cbr")).exists(), "{name}");
        let converted = media(&pool, id).await;
        assert_eq!(
            converted,
            (
                "READY".to_owned(),
                Some("application/zip".to_owned()),
                None,
                3
            )
        );
    }
    let after = pages(&pool, &four).await;
    let names = |pages: &[db::content::MediaPage]| -> Vec<String> {
        pages.iter().map(|page| page.0.clone()).collect()
    };
    assert_eq!(names(&after), names(&before));
    assert_eq!(after[0].4, "kept", "the page hash follows the page");
    let read: Vec<i32> = sqlx::query_scalar("SELECT PAGE FROM READ_PROGRESS WHERE BOOK_ID = ?")
        .bind(&four)
        .fetch_all(&pool)
        .await
        .expect("Progress");
    assert_eq!(read, [2]);
    assert_eq!(leftovers(&series), Vec::<String>::new());
    server.stop().await;
}

#[tokio::test]
async fn conversions_leave_files_alone_when_disabled_clashing_or_changed() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/clash.cbr", rar("rar4-solid.rar")),
            ("Series/clash.cbz", b"not an archive".to_vec()),
            ("Series/changed.cbr", rar("rar4-solid.rar")),
            ("Series/kept.cbr", rar("rar5-solid.rar")),
        ],
    )
    .await;
    let series = directory.path().join("comics/Series");
    let (clash, _) = book_at(&pool, "clash.cbr").await;
    let (changed, _) = book_at(&pool, "changed.cbr").await;
    let (kept, series_id) = book_at(&pool, "kept.cbr").await;

    // Queued while the library does not convert: nothing happens.
    server.stop().await;
    queue_kotlin_conversion(directory.path(), &kept, &series_id).await;
    let server = TestServer::start(directory.path()).await;
    wait_for_conversions(directory.path()).await;
    assert!(url(&pool, &kept).await.ends_with("kept.cbr"));
    assert!(series.join("kept.cbr").is_file());

    fs::write(series.join("changed.cbr"), rar("rar5-solid.rar")).expect("Changed book");
    convert(&server, directory.path(), &library).await;

    assert!(
        url(&pool, &kept).await.ends_with("kept.cbz"),
        "a book of the library is converted"
    );
    assert!(url(&pool, &clash).await.ends_with("clash.cbr"));
    assert_eq!(
        fs::read(series.join("clash.cbz")).expect("Existing CBZ"),
        b"not an archive"
    );
    assert_eq!(
        fs::read(series.join("clash.cbr")).expect("Kept RAR"),
        rar("rar4-solid.rar")
    );
    assert!(url(&pool, &changed).await.ends_with("changed.cbr"));
    assert_eq!(
        fs::read(series.join("changed.cbr")).expect("Changed RAR"),
        rar("rar5-solid.rar")
    );
    assert!(!series.join("changed.cbz").exists());
    assert_eq!(leftovers(&series), Vec::<String>::new());
    server.stop().await;
}

fn comic() -> Vec<u8> {
    cbz::zip(&[("1.jpg", cbz::STORED, &cbz::fixture(JPEG))])
}

#[tokio::test]
async fn misnamed_books_get_the_extension_of_their_type_keeping_their_state() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let root = directory.path().join("comics");
    for (path, content) in [
        ("Series/zipped.cbr", comic()),
        ("Series/rared.cbz", rar("rar4-solid.rar")),
        ("Series/plain.epub", comic()),
        ("_oneshots/One.cbr", comic()),
    ] {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("Book directory")).expect("Book directory");
        fs::write(path, content).expect("Book file");
    }
    let body = json!({"name": "Comics", "root": root, "oneshotsDirectory": "_oneshots"});
    let library = admin_json(
        &server,
        Method::POST,
        "/api/v1/libraries",
        Some(&body),
        StatusCode::OK,
    )
    .await;
    wait_for_scan_work(directory.path()).await;
    let pool = db::main_pool(directory.path()).await;
    let (zipped, _) = book_at(&pool, "zipped.cbr").await;
    let hash = |id: String| {
        let pool = &pool;
        async move {
            let query = sqlx::query_scalar("SELECT FILE_HASH FROM BOOK WHERE ID = ?");
            let hash: String = query.bind(id).fetch_one(pool).await.expect("Hash");
            hash
        }
    };
    let before = hash(zipped.clone()).await;

    enable(&server, directory.path(), &library, "repairExtensions").await;

    assert!(url(&pool, &zipped).await.ends_with("Series/zipped.cbz"));
    assert_eq!(hash(zipped).await, before, "the content is unchanged");
    let (rared, _) = book_at(&pool, "rared.cbr").await;
    assert_eq!(media(&pool, &rared).await.0, "READY");
    let (one, oneshot) = book_at(&pool, "One.cbz").await;
    let series_url: String = sqlx::query_scalar("SELECT URL FROM SERIES WHERE ID = ?")
        .bind(&oneshot)
        .fetch_one(&pool)
        .await
        .expect("Series URL");
    assert_eq!(series_url, url(&pool, &one).await, "a oneshot is its book");
    book_at(&pool, "plain.epub").await;
    let series = root.join("Series");
    let mut names: Vec<_> = fs::read_dir(&series)
        .expect("Series directory")
        .map(|entry| {
            entry
                .expect("Entry")
                .file_name()
                .into_string()
                .expect("UTF-8")
        })
        .collect();
    names.sort();
    assert_eq!(names, ["plain.epub", "rared.cbr", "zipped.cbz"]);
    assert_eq!(leftovers(&root.join("_oneshots")), Vec::<String>::new());
    server.stop().await;
}

#[tokio::test]
async fn repairs_leave_files_alone_when_disabled_clashing_or_changed() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/clash.cbr", comic()),
            ("Series/clash.cbz", b"not an archive".to_vec()),
            ("Series/changed.cbr", comic()),
            ("Series/kept.cbr", comic()),
        ],
    )
    .await;
    let series = directory.path().join("comics/Series");
    let (clash, _) = book_at(&pool, "clash.cbr").await;
    let (changed, _) = book_at(&pool, "changed.cbr").await;
    let (kept, series_id) = book_at(&pool, "kept.cbr").await;

    // Queued while the library does not repair: nothing happens.
    server.stop().await;
    queue_kotlin_repair(directory.path(), &kept, &series_id).await;
    let server = TestServer::start(directory.path()).await;
    wait_for_conversions(directory.path()).await;
    assert!(url(&pool, &kept).await.ends_with("kept.cbr"));

    let jpeg = cbz::fixture(JPEG);
    let rewritten = cbz::zip(&[("1.jpg", cbz::STORED, &jpeg), ("2.jpg", cbz::STORED, &jpeg)]);
    fs::write(series.join("changed.cbr"), &rewritten).expect("Changed book");
    enable(&server, directory.path(), &library, "repairExtensions").await;

    assert!(
        url(&pool, &kept).await.ends_with("kept.cbz"),
        "a book of the library is repaired"
    );
    assert!(url(&pool, &clash).await.ends_with("clash.cbr"));
    assert_eq!(
        fs::read(series.join("clash.cbz")).expect("Existing file"),
        b"not an archive"
    );
    assert_eq!(
        fs::read(series.join("clash.cbr")).expect("Kept book"),
        comic()
    );
    assert!(url(&pool, &changed).await.ends_with("changed.cbr"));
    assert_eq!(
        fs::read(series.join("changed.cbr")).expect("Changed book"),
        rewritten
    );
    assert!(!series.join("changed.cbz").exists());
    assert_eq!(leftovers(&series), Vec::<String>::new());
    server.stop().await;
}
