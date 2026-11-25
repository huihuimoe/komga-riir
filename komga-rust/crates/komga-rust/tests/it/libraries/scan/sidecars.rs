//! Sidecars found by library scans: stored, refreshed when new or changed, and forgotten.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use komga_test_support::TestDir;
use komga_types::FileUrl;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::super::admin_json;
use super::{scan, touch};
use crate::support::TestServer;
use crate::support::db::{self, content};
use crate::support::tasks::{clear_after_scans, wait_for_scan_work};

/// `(URL, PARENT_URL, LAST_MODIFIED_TIME)` with URLs relative to the root, sorted by URL.
async fn sidecars(
    pool: &SqlitePool,
    root: &Path,
    library: &Value,
) -> Vec<(String, String, String)> {
    let rows = content::sidecars(pool).await;
    let base = FileUrl::file(root).to_string();
    let relative = |url: &str| url.strip_prefix(&base).expect("Under the root").to_owned();
    rows.into_iter()
        .map(|(url, parent, time, library_id)| {
            assert_eq!(library["id"], json!(library_id));
            (relative(&url), relative(&parent), time)
        })
        .collect()
}

fn set_modified_later(path: &Path) {
    File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now() + Duration::from_secs(60)))
        .expect("Sidecar time");
}

/// A library whose `Series A` holds the book `a.cbz` with the sidecars `a-1.png`, `cover.jpg`
/// and `series.json`, after the creation scan.
struct Fixture {
    server: TestServer,
    root: PathBuf,
    library: Value,
    pool: SqlitePool,
    /// The series whose metadata the creation scan refreshed.
    created: Vec<String>,
    series_id: String,
    directory: TestDir,
}

impl Fixture {
    async fn new() -> Self {
        let directory = komga_test_support::tempdir().expect("Data directory");
        let server = TestServer::start_claimed(directory.path()).await;
        let root = directory.path().join("comics");
        for file in [
            "Series A/a.cbz",
            "Series A/a-1.png",
            "Series A/cover.jpg",
            "Series A/series.json",
            "No books/cover.jpg",
        ] {
            touch(&root.join(file));
        }
        let body = json!({"name": "Comics", "root": root});
        let library = admin_json(
            &server,
            Method::POST,
            "/api/v1/libraries",
            Some(&body),
            StatusCode::OK,
        )
        .await;
        let pool = db::main_pool(directory.path()).await;
        wait_for_scan_work(directory.path()).await;
        let created = content::aggregated_since(&pool, &[]).await;
        let [(_, series_id)] =
            <[_; 1]>::try_from(content::book_series(&pool).await).expect("One book");
        Self {
            series_id,
            directory,
            server,
            root,
            library,
            pool,
            created,
        }
    }

    fn series_directory(&self) -> PathBuf {
        self.root.join("Series A")
    }

    async fn sidecars(&self) -> Vec<(String, String, String)> {
        sidecars(&self.pool, &self.root, &self.library).await
    }

    /// Clears the queue, scans, and returns the series whose metadata the scan refreshed, which
    /// the aggregation that ends each series metadata refresh shows.
    async fn rescan(&self, query: &str) -> Vec<String> {
        clear_after_scans(&self.server, self.directory.path()).await;
        let before = content::aggregation_dates(&self.pool).await;
        scan(&self.server, &self.library, query).await;
        wait_for_scan_work(self.directory.path()).await;
        content::aggregated_since(&self.pool, &before).await
    }
}

#[tokio::test]
async fn a_scan_stores_sidecars_and_repeated_scans_queue_no_refresh() {
    let fixture = Fixture::new().await;
    assert_eq!(fixture.created, std::slice::from_ref(&fixture.series_id));
    let stored = fixture.sidecars().await;
    let urls: Vec<(&str, &str)> = stored
        .iter()
        .map(|row| (row.0.as_str(), row.1.as_str()))
        .collect();
    assert_eq!(
        urls,
        [
            ("/Series%20A/a-1.png", "/Series%20A/a.cbz"),
            ("/Series%20A/cover.jpg", "/Series%20A/"),
            ("/Series%20A/series.json", "/Series%20A/"),
        ],
        "a directory without books has no sidecars"
    );

    // A deep scan refreshes every series metadata, sidecar or not.
    for (query, expected) in [
        ("", vec![]),
        ("?deep=true", vec![fixture.series_id.clone()]),
    ] {
        assert_eq!(fixture.rescan(query).await, expected, "{query}");
        assert_eq!(fixture.sidecars().await, stored);
    }
    fixture.server.stop().await;
}

#[tokio::test]
async fn changed_and_replaced_sidecars_queue_refreshes_and_removed_ones_are_forgotten() {
    let fixture = Fixture::new().await;
    let stored = fixture.sidecars().await;
    let series_directory = fixture.series_directory();

    // Touching files leaves the directory time alone, so only the sidecars queue refreshes.
    set_modified_later(&series_directory.join("cover.jpg"));
    set_modified_later(&series_directory.join("series.json"));
    assert_eq!(
        fixture.rescan("").await,
        std::slice::from_ref(&fixture.series_id)
    );
    let changed = fixture.sidecars().await;
    assert_eq!(changed[0], stored[0]);
    assert!(
        changed[1..]
            .iter()
            .zip(&stored[1..])
            .all(|(new, old)| new.2 != old.2),
        "{changed:?}"
    );

    // Replacing the book sidecar changes the directory, which also refreshes the series.
    fs::remove_file(series_directory.join("a-1.png")).expect("Remove sidecar");
    touch(&series_directory.join("A-2.JPG"));
    assert_eq!(
        fixture.rescan("").await,
        std::slice::from_ref(&fixture.series_id)
    );
    let replaced = fixture.sidecars().await;
    assert_eq!(
        (replaced[0].0.as_str(), replaced[0].1.as_str()),
        ("/Series%20A/A-2.JPG", "/Series%20A/a.cbz")
    );
    assert_eq!(
        replaced[1..],
        changed[1..],
        "the removed sidecar is forgotten"
    );

    assert_eq!(fixture.rescan("").await, Vec::<String>::new());
    assert_eq!(fixture.sidecars().await, replaced);
    fixture.server.stop().await;
}
