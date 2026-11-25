//! Local artwork: sidecar images stored as book and series thumbnails by refresh tasks, served
//! without reading the library, and dropped or replaced by later scans.

use std::fs::{self, File};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use image::{ImageFormat, Rgb, RgbImage};
use komga_test_support::TestDir;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, SqlitePool};

use super::library_with_books;
use super::reading::{Fetched, fetch};
use super::thumbnails::{book_with_first_page, cover};
use crate::libraries::route;
use crate::support::TestServer;
use crate::support::db::content::book;
use crate::support::tasks::{TaskRow, wait_for_tasks};

/// `(URL file name, TYPE, SELECTED, MEDIA_TYPE, WIDTH, HEIGHT, FILE_SIZE, LENGTH(THUMBNAIL), ID)`
/// of the book's or series' thumbnails, by URL.
type Row = (
    String,
    String,
    bool,
    String,
    i64,
    i64,
    i64,
    Option<i64>,
    String,
);

fn image(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
    let image = RgbImage::from_pixel(
        width,
        height,
        Rgb([u8::try_from(width).unwrap_or(u8::MAX), 0, 0]),
    );
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), format)
        .expect("Encode artwork");
    bytes
}

struct Fixture {
    server: TestServer,
    library: Value,
    pool: SqlitePool,
    book_id: String,
    series_id: String,
    directory: TestDir,
}

impl Fixture {
    /// `Series A` holds `a.cbz`, the book sidecars `a-1.JPG` (8 x 4) and `a.png` (6 x 3), a
    /// `a-2.png` that is not an image, and the series covers `Cover.PNG` (10 x 5) and `folder.jpg`.
    async fn new() -> Self {
        let directory = komga_test_support::tempdir().expect("Data directory");
        let (server, library, pool) = library_with_books(
            directory.path(),
            &[
                ("Series A/a.cbz", book_with_first_page(&cover())),
                ("Series A/a-1.JPG", image(8, 4, ImageFormat::Jpeg)),
                ("Series A/a.png", image(6, 3, ImageFormat::Png)),
                ("Series A/a-2.png", b"not an image".to_vec()),
                ("Series A/Cover.PNG", image(10, 5, ImageFormat::Png)),
                ("Series A/folder.jpg", image(12, 6, ImageFormat::Jpeg)),
            ],
        )
        .await;
        let (book_id, series_id) = book(&pool, "a").await;
        let fixture = Self {
            directory,
            server,
            library,
            pool,
            book_id,
            series_id,
        };
        fixture.wait().await;
        fixture
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join("comics/Series A").join(name)
    }

    /// Waits for the thumbnail generations and artwork refreshes that ran or failed.
    async fn wait(&self) -> Vec<TaskRow> {
        wait_for_tasks(self.directory.path(), |rows| {
            rows.iter()
                .filter(|row| {
                    row.0.starts_with("GENERATE_BOOK_THUMBNAIL_")
                        || row.0.contains("_LOCAL_ARTWORK_")
                        || row.0.starts_with("SCAN_LIBRARY_")
                })
                .all(|row| row.3.contains("\"eraFailed\""))
        })
        .await
    }

    async fn rescan(&self) {
        let response = self
            .server
            .admin(Method::POST, &format!("{}/scan", route(&self.library)))
            .send()
            .await
            .expect("Scan request");
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        self.wait().await;
    }

    async fn thumbnail(&self) -> Fetched {
        let route = format!("/api/v1/books/{}/thumbnail", self.book_id);
        fetch(self.server.admin(Method::GET, &route)).await
    }

    async fn series_thumbnail(&self) -> Fetched {
        let route = format!("/api/v1/series/{}/thumbnail", self.series_id);
        fetch(self.server.admin(Method::GET, &route)).await
    }

    async fn rows(&self, table: &str, owner: &str, id: &str) -> Vec<Row> {
        let rows: Vec<Row> = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT COALESCE(URL, ''), TYPE, SELECTED, MEDIA_TYPE, WIDTH, HEIGHT, FILE_SIZE, \
             LENGTH(THUMBNAIL), ID FROM {table} WHERE {owner} = ? ORDER BY URL"
        )))
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .expect("Thumbnail rows");
        rows.into_iter()
            .map(|mut row| {
                row.0 = row.0.rsplit('/').next().unwrap_or_default().to_owned();
                row
            })
            .collect()
    }

    async fn book_rows(&self) -> Vec<Row> {
        self.rows("THUMBNAIL_BOOK", "BOOK_ID", &self.book_id).await
    }

    async fn series_rows(&self) -> Vec<Row> {
        self.rows("THUMBNAIL_SERIES", "SERIES_ID", &self.series_id)
            .await
    }
}

/// `(file name, TYPE, SELECTED, MEDIA_TYPE, WIDTH, HEIGHT)` and whether the stored bytes are the
/// whole file of `FILE_SIZE` bytes.
fn summary(rows: &[Row]) -> Vec<(&str, &str, bool, &str, i64, i64, bool)> {
    rows.iter()
        .map(|row| {
            let whole = row.7 == Some(row.6);
            (&*row.0, &*row.1, row.2, &*row.3, row.4, row.5, whole)
        })
        .collect()
}

fn set_modified_later(path: &Path) {
    File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_modified(SystemTime::now() + Duration::from_secs(60)))
        .expect("Artwork time");
}

#[tokio::test]
async fn local_artwork_is_stored_selected_and_follows_scans() {
    let fixture = Fixture::new().await;
    assert_eq!(
        summary(&fixture.book_rows().await),
        [
            ("", "GENERATED", false, "image/jpeg", 300, 150, true),
            ("a-1.JPG", "SIDECAR", true, "image/jpeg", 8, 4, true),
            ("a.png", "SIDECAR", false, "image/png", 6, 3, true),
        ],
        "the first artwork file by name is selected over the generated thumbnail"
    );
    assert_eq!(
        summary(&fixture.series_rows().await),
        [
            ("Cover.PNG", "SIDECAR", true, "image/png", 10, 5, true),
            ("folder.jpg", "SIDECAR", false, "image/jpeg", 12, 6, true),
        ]
    );
    let series_cover = fs::read(fixture.path("Cover.PNG")).expect("Series cover");
    let poster = fixture.series_thumbnail().await;
    assert_eq!(poster.status, StatusCode::OK);
    assert!(poster.body == series_cover, "the series cover as stored");
    let artwork = fs::read(fixture.path("a-1.JPG")).expect("Artwork");
    let thumbnail = fixture.thumbnail().await;
    assert_eq!(thumbnail.status, StatusCode::OK);
    assert!(thumbnail.body == artwork, "the artwork file as stored");

    // A hit reads the database only: the moved artwork file is not needed.
    let moved = fixture.directory.path().join("a-1.moved");
    fs::rename(fixture.path("a-1.JPG"), &moved).expect("Move the artwork away");
    assert!(
        fixture.thumbnail().await.body == artwork,
        "served while moved"
    );

    // A scan that finds it gone drops it and selects another thumbnail.
    let series_before = fixture.series_rows().await;
    fixture.rescan().await;
    let rows = fixture.book_rows().await;
    assert_eq!(
        summary(&rows)
            .iter()
            .map(|row| (row.0, row.2))
            .collect::<Vec<_>>(),
        [("", false), ("a.png", true)]
    );
    let other = fs::read(fixture.path("a.png")).expect("Artwork");
    assert!(fixture.thumbnail().await.body == other, "the other artwork");
    assert_eq!(fixture.series_rows().await, series_before, "untouched");

    // A changed file is read again by the refresh its scan queues.
    let changed = image(14, 7, ImageFormat::Png);
    fs::write(fixture.path("a.png"), &changed).expect("Change the artwork");
    set_modified_later(&fixture.path("a.png"));
    fixture.rescan().await;
    let refreshed = fixture.book_rows().await;
    assert_eq!(
        summary(&refreshed)[1],
        ("a.png", "SIDECAR", true, "image/png", 14, 7, true)
    );
    assert_ne!(refreshed[1].8, rows[1].8, "a new row");
    assert!(fixture.thumbnail().await.body == changed, "the new content");

    // Removing the series cover selects the remaining one.
    fs::remove_file(fixture.path("Cover.PNG")).expect("Remove the series cover");
    fixture.rescan().await;
    let series = fixture.series_rows().await;
    assert_eq!(
        summary(&series)
            .iter()
            .map(|row| (row.0, row.2))
            .collect::<Vec<_>>(),
        [("folder.jpg", true)]
    );
    assert_eq!(series[0].8, series_before[1].8, "the same row");
    let folder = fs::read(fixture.path("folder.jpg")).expect("Series cover");
    assert!(
        fixture.series_thumbnail().await.body == folder,
        "the other cover"
    );

    // Without series covers, the series shows its book's thumbnail.
    fs::remove_file(fixture.path("folder.jpg")).expect("Remove the series cover");
    fixture.rescan().await;
    assert!(fixture.series_rows().await.is_empty());
    assert!(
        fixture.series_thumbnail().await.body == changed,
        "the book's thumbnail"
    );
    fixture.server.stop().await;
}

#[tokio::test]
async fn libraries_can_skip_local_artwork() {
    let fixture = Fixture::new().await;
    let response = fixture
        .server
        .admin(Method::PATCH, &route(&fixture.library))
        .json(&json!({"importLocalArtwork": false}))
        .send()
        .await
        .expect("Library update");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let before = fixture.book_rows().await;

    // The refresh a new sidecar queues leaves the stored thumbnails alone.
    fs::write(fixture.path("a-3.png"), image(4, 2, ImageFormat::Png)).expect("New artwork");
    fixture.rescan().await;
    assert_eq!(fixture.book_rows().await, before);
    fixture.server.stop().await;
}

#[tokio::test]
async fn kotlin_artwork_rows_without_bytes_are_read_once() {
    let fixture = Fixture::new().await;
    let store = |url: String, id: &'static str| {
        store_kotlin_row(
            &fixture.pool,
            "THUMBNAIL_BOOK",
            "BOOK_ID",
            &fixture.book_id,
            url,
            id,
        )
    };
    let url: String = sqlx::query_scalar("SELECT URL FROM THUMBNAIL_BOOK WHERE URL LIKE '%/a.png'")
        .fetch_one(&fixture.pool)
        .await
        .expect("Artwork URL");

    store(url.clone(), "KOTLIN1").await;
    let artwork = fs::read(fixture.path("a.png")).expect("Artwork");
    assert!(
        fixture.thumbnail().await.body == artwork,
        "read from the file"
    );
    let length: Option<i64> =
        sqlx::query_scalar("SELECT LENGTH(THUMBNAIL) FROM THUMBNAIL_BOOK WHERE ID = 'KOTLIN1'")
            .fetch_one(&fixture.pool)
            .await
            .expect("Cached row");
    assert_eq!(length, i64::try_from(artwork.len()).ok(), "and stored");

    // A row whose file is gone is dropped, and another thumbnail served.
    store(url.replace("/a.png", "/gone.png"), "KOTLIN2").await;
    let fallback = fixture.thumbnail().await;
    assert_eq!(fallback.status, StatusCode::OK);
    let ids: Vec<String> = fixture
        .book_rows()
        .await
        .into_iter()
        .map(|row| row.8)
        .collect();
    assert!(!ids.contains(&"KOTLIN2".to_owned()), "{ids:?}");
    assert!(ids.contains(&"KOTLIN1".to_owned()), "{ids:?}");

    // Series rows alike.
    let url: String =
        sqlx::query_scalar("SELECT URL FROM THUMBNAIL_SERIES WHERE URL LIKE '%/folder.jpg'")
            .fetch_one(&fixture.pool)
            .await
            .expect("Series cover URL");
    let series_id = &fixture.series_id;
    store_kotlin_row(
        &fixture.pool,
        "THUMBNAIL_SERIES",
        "SERIES_ID",
        series_id,
        url,
        "KOTLIN3",
    )
    .await;
    let folder = fs::read(fixture.path("folder.jpg")).expect("Series cover");
    assert!(
        fixture.series_thumbnail().await.body == folder,
        "read from the file"
    );
    let length: Option<i64> =
        sqlx::query_scalar("SELECT LENGTH(THUMBNAIL) FROM THUMBNAIL_SERIES WHERE ID = 'KOTLIN3'")
            .fetch_one(&fixture.pool)
            .await
            .expect("Cached row");
    assert_eq!(length, i64::try_from(folder.len()).ok(), "and stored");
    fixture.server.stop().await;
}

/// Selects the row Kotlin's `LocalArtworkProvider` stores: the URL without the bytes.
async fn store_kotlin_row(
    pool: &SqlitePool,
    table: &str,
    owner: &str,
    owner_id: &str,
    url: String,
    id: &str,
) {
    sqlx::query(AssertSqlSafe(format!(
        "UPDATE {table} SET SELECTED = 0 WHERE {owner} = ?"
    )))
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("Unselect");
    sqlx::query(AssertSqlSafe(format!(
        "INSERT INTO {table} (ID, {owner}, URL, SELECTED, TYPE, MEDIA_TYPE) \
         VALUES (?, ?, ?, 1, 'SIDECAR', 'image/png')"
    )))
    .bind(id)
    .bind(owner_id)
    .bind(url)
    .execute(pool)
    .await
    .expect("Kotlin row");
}
