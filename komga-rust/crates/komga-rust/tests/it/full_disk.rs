//! Writes to a full disk. The test needs `KOMGA_TEST_FULL_DISK`, a directory on a small Linux
//! filesystem it may fill, such as a tmpfs of 32 MiB:
//! `KOMGA_TEST_FULL_DISK=/full cargo nextest run --run-ignored only -- full_disk::`.

use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use image::{ImageFormat, RgbImage};
use komga_rust::config::AppConfig;
use komga_types::SearchEntity;
use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use crate::support::cbz::{self, STORED};
use crate::support::posters::{file, list_at};
use crate::support::search::{hits, pending, wait_for_index};
use crate::support::tasks::wait_for_scan_work;
use crate::support::{TestServer, app_config, db};

/// A PNG of noise of about 480 KB, as it does not compress.
fn noise(seed: u32) -> Vec<u8> {
    let mut state = seed;
    let image = RgbImage::from_fn(400, 400, |_, _| {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let [r, g, b, _] = state.to_le_bytes();
        image::Rgb([r, g, b])
    });
    let mut bytes = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("Encode poster");
    bytes
}

/// Appends to `filler` until the filesystem has no room left, then frees `room` bytes of it.
fn fill_all_but(filler: &Path, room: u64) {
    let mut file = File::options()
        .create(true)
        .append(true)
        .open(filler)
        .expect("Filler");
    let chunk = vec![0; 64 * 1024];
    loop {
        if let Err(error) = file.write_all(&chunk).and_then(|()| file.sync_all()) {
            assert_eq!(error.kind(), ErrorKind::StorageFull, "{error}");
            let length = file.metadata().expect("Filler length").len();
            file.set_len(length - room).expect("Free room");
            return;
        }
    }
}

fn config(data: &Path) -> AppConfig {
    let mut config = app_config(data);
    // Long enough to fill the disk between a change and its index update.
    config.lucene.commit_delay.value = Duration::from_secs(2);
    config
}

/// The series `Watchmen` in a library outside the full disk, scanned and indexed.
async fn library(server: &TestServer, data: &Path, root: &Path) -> (SqlitePool, String) {
    fs::create_dir_all(root.join("Watchmen")).expect("Series directory");
    let book = cbz::zip(&[("1.png", STORED, b"png")]);
    fs::write(root.join("Watchmen/Watchmen 1.cbz"), book).expect("Book file");
    let response = server
        .admin(Method::POST, "/api/v1/libraries")
        .json(&json!({"name": "Comics", "root": root}))
        .send()
        .await
        .expect("Library creation");
    assert_eq!(response.status(), StatusCode::OK);
    wait_for_scan_work(data).await;
    let pool = db::main_pool(data).await;
    wait_for_index(&pool).await;
    let series = sqlx::query_scalar("SELECT ID FROM SERIES")
        .fetch_one(&pool)
        .await
        .expect("Series");
    (pool, series)
}

async fn upload(server: &TestServer, series: &str, poster: Vec<u8>) -> StatusCode {
    server
        .admin(Method::POST, &format!("/api/v1/series/{series}/thumbnails"))
        .multipart(file(poster))
        .send()
        .await
        .expect("Upload")
        .status()
}

async fn posters(server: &TestServer, series: &str) -> Vec<serde_json::Value> {
    list_at(server, &format!("/api/v1/series/{series}/thumbnails")).await
}

/// The data directory on the full disk, emptied.
fn data_directory() -> PathBuf {
    let full = std::env::var_os("KOMGA_TEST_FULL_DISK").expect("KOMGA_TEST_FULL_DISK");
    let data = Path::new(&full).join("data");
    if data.exists() {
        fs::remove_dir_all(&data).expect("Empty the data directory");
    }
    fs::create_dir(&data).expect("Data directory");
    data
}

#[tokio::test]
#[ignore = "needs KOMGA_TEST_FULL_DISK, a small filesystem it fills"]
async fn writes_to_a_full_disk_fail_and_keep_published_posters_and_index() {
    let data = data_directory();
    let library_root = komga_test_support::tempdir().expect("Library root");
    let server = TestServer::start_claimed_with(config(&data)).await;
    let (pool, series) = library(&server, &data, library_root.path()).await;
    assert_eq!(upload(&server, &series, noise(0)).await, StatusCode::OK);
    let published = posters(&server, &series).await;

    // Room for small writes, such as selecting a poster, but not for the poster itself.
    let filler = data.join("filler");
    fill_all_but(&filler, 256 * 1024);
    assert_eq!(
        upload(&server, &series, noise(1)).await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(posters(&server, &series).await, published);

    // The rename is committed; its index update comes once the disk is full.
    let response = server
        .admin(Method::PATCH, &format!("/api/v1/series/{series}/metadata"))
        .json(&json!({"title": "Rorschach"}))
        .send()
        .await
        .expect("Rename");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    fill_all_but(&filler, 0);
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(pending(&pool).await > 0, "the index update failed");
    server.stop().await;
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .expect("Integrity check");
    assert_eq!(integrity, "ok");
    assert_eq!(
        hits(&data, SearchEntity::Series, "watchmen"),
        [series.as_str()]
    );

    fs::remove_file(&filler).expect("Free the disk");
    let server = TestServer::start_with(config(&data)).await;
    wait_for_index(&pool).await;
    assert_eq!(posters(&server, &series).await, published);
    assert_eq!(upload(&server, &series, noise(99)).await, StatusCode::OK);
    server.stop().await;
    assert_eq!(
        hits(&data, SearchEntity::Series, "rorschach"),
        [series.as_str()]
    );
}
