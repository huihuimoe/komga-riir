//! The series, books, media and sidecars scans and analyses store.

use sqlx::SqlitePool;

/// `(ID, NAME, BOOK_COUNT, deleted)` of the series, sorted by name.
pub async fn series(pool: &SqlitePool) -> Vec<(String, String, i32, bool)> {
    sqlx::query_as(
        "SELECT ID, NAME, BOOK_COUNT, DELETED_DATE IS NOT NULL FROM SERIES ORDER BY NAME",
    )
    .fetch_all(pool)
    .await
    .expect("Read series")
}

/// `(ID, NAME, URL, BOOK_COUNT, ONESHOT)` of the series, sorted by name.
pub async fn series_with_urls(pool: &SqlitePool) -> Vec<(String, String, String, i32, bool)> {
    sqlx::query_as("SELECT ID, NAME, URL, BOOK_COUNT, ONESHOT FROM SERIES ORDER BY NAME")
        .fetch_all(pool)
        .await
        .expect("Read series")
}

/// `(ID, NAME, NUMBER, deleted)` of the books, sorted by name.
pub async fn books(pool: &SqlitePool) -> Vec<(String, String, i32, bool)> {
    sqlx::query_as("SELECT ID, NAME, NUMBER, DELETED_DATE IS NOT NULL FROM BOOK ORDER BY NAME")
        .fetch_all(pool)
        .await
        .expect("Read books")
}

/// `(ID, NAME, NUMBER, METADATA NUMBER, deleted)` of the books, sorted by name.
pub async fn numbered_books(pool: &SqlitePool) -> Vec<(String, String, i32, String, bool)> {
    sqlx::query_as(
        "SELECT b.ID, b.NAME, b.NUMBER, d.NUMBER, b.DELETED_DATE IS NOT NULL \
         FROM BOOK b JOIN BOOK_METADATA d ON d.BOOK_ID = b.ID ORDER BY b.NAME",
    )
    .fetch_all(pool)
    .await
    .expect("Read books")
}

/// `(ID, NAME, URL, NUMBER, FILE_SIZE, METADATA NUMBER, METADATA NUMBER_SORT, MEDIA STATUS)`.
pub type ScannedBook = (String, String, String, i32, i64, String, f64, String);

/// Books sorted by series name and number.
pub async fn scanned_books(pool: &SqlitePool) -> Vec<ScannedBook> {
    sqlx::query_as(
        "SELECT b.ID, b.NAME, b.URL, b.NUMBER, b.FILE_SIZE, d.NUMBER, d.NUMBER_SORT, m.STATUS \
         FROM BOOK b JOIN BOOK_METADATA d ON d.BOOK_ID = b.ID JOIN MEDIA m ON m.BOOK_ID = b.ID \
         JOIN SERIES s ON s.ID = b.SERIES_ID ORDER BY s.NAME, b.NUMBER",
    )
    .fetch_all(pool)
    .await
    .expect("Read books")
}

/// `(ID, SERIES_ID)` of the books.
pub async fn book_series(pool: &SqlitePool) -> Vec<(String, String)> {
    sqlx::query_as("SELECT ID, SERIES_ID FROM BOOK")
        .fetch_all(pool)
        .await
        .expect("Read books")
}

/// `(ID, SERIES_ID)` of the book named `name`.
pub async fn book(pool: &SqlitePool, name: &str) -> (String, String) {
    sqlx::query_as("SELECT ID, SERIES_ID FROM BOOK WHERE NAME = ?")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("Stored book")
}

/// `(FILE_LAST_MODIFIED, FILE_SIZE)` the scan recorded for a book.
pub async fn book_file(pool: &SqlitePool, book_id: &str) -> (String, i64) {
    sqlx::query_as("SELECT FILE_LAST_MODIFIED, FILE_SIZE FROM BOOK WHERE ID = ?")
        .bind(book_id)
        .fetch_one(pool)
        .await
        .expect("Stored book")
}

/// `(FILE_HASH, FILE_HASH_KOREADER)` of a book.
pub async fn book_hashes(pool: &SqlitePool, book_id: &str) -> (String, String) {
    sqlx::query_as("SELECT FILE_HASH, FILE_HASH_KOREADER FROM BOOK WHERE ID = ?")
        .bind(book_id)
        .fetch_one(pool)
        .await
        .expect("Stored book")
}

/// `LAST_MODIFIED_DATE` of a book's media, which every media update stamps.
pub async fn media_last_modified(pool: &SqlitePool, book_id: &str) -> String {
    sqlx::query_scalar("SELECT LAST_MODIFIED_DATE FROM MEDIA WHERE BOOK_ID = ?")
        .bind(book_id)
        .fetch_one(pool)
        .await
        .expect("Stored media")
}

/// `(ID, DELETED_DATE)` of the deleted books and series, sorted by ID.
pub async fn deletion_dates(pool: &SqlitePool) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT ID, DELETED_DATE FROM BOOK WHERE DELETED_DATE IS NOT NULL \
         UNION ALL SELECT ID, DELETED_DATE FROM SERIES WHERE DELETED_DATE IS NOT NULL \
         ORDER BY ID",
    )
    .fetch_all(pool)
    .await
    .expect("Read deletion dates")
}

/// `(STATUS, MEDIA_TYPE, COMMENT, PAGE_COUNT)` of a book.
pub type Media = (String, Option<String>, Option<String>, i32);

pub async fn media(pool: &SqlitePool, book_id: &str) -> Media {
    sqlx::query_as("SELECT STATUS, MEDIA_TYPE, COMMENT, PAGE_COUNT FROM MEDIA WHERE BOOK_ID = ?")
        .bind(book_id)
        .fetch_one(pool)
        .await
        .expect("Stored media")
}

/// `(FILE_NAME, MEDIA_TYPE, WIDTH, HEIGHT, FILE_HASH, FILE_SIZE)` in page order.
pub type MediaPage = (
    String,
    String,
    Option<i32>,
    Option<i32>,
    String,
    Option<i64>,
);

pub async fn pages(pool: &SqlitePool, book_id: &str) -> Vec<MediaPage> {
    sqlx::query_as(
        "SELECT FILE_NAME, MEDIA_TYPE, WIDTH, HEIGHT, FILE_HASH, FILE_SIZE FROM MEDIA_PAGE \
         WHERE BOOK_ID = ? ORDER BY NUMBER",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .expect("Stored pages")
}

/// `(FILE_NAME, MEDIA_TYPE, FILE_SIZE)` in insertion order.
pub type MediaFile = (String, Option<String>, Option<i64>);

pub async fn files(pool: &SqlitePool, book_id: &str) -> Vec<MediaFile> {
    sqlx::query_as(
        "SELECT FILE_NAME, MEDIA_TYPE, FILE_SIZE FROM MEDIA_FILE WHERE BOOK_ID = ? ORDER BY ROWID",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .expect("Stored files")
}

/// `(URL, PARENT_URL, LAST_MODIFIED_TIME, LIBRARY_ID)` of the sidecars, sorted by URL.
pub async fn sidecars(pool: &SqlitePool) -> Vec<(String, String, String, String)> {
    sqlx::query_as(
        "SELECT URL, PARENT_URL, LAST_MODIFIED_TIME, LIBRARY_ID FROM SIDECAR ORDER BY URL",
    )
    .fetch_all(pool)
    .await
    .expect("Read sidecars")
}

/// `(SERIES_ID, LAST_MODIFIED_DATE)` of the book metadata aggregations, sorted by series.
pub async fn aggregation_dates(pool: &SqlitePool) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT SERIES_ID, LAST_MODIFIED_DATE FROM BOOK_METADATA_AGGREGATION ORDER BY SERIES_ID",
    )
    .fetch_all(pool)
    .await
    .expect("Read aggregation dates")
}

/// The series whose book metadata aggregation ran since `before`, an [`aggregation_dates`]
/// snapshot, sorted. A series missing from `before` counts once updated after its insert: the
/// insert's `CURRENT_TIMESTAMP` has no fraction, unlike the date an update writes.
pub async fn aggregated_since(pool: &SqlitePool, before: &[(String, String)]) -> Vec<String> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT SERIES_ID, CREATED_DATE, LAST_MODIFIED_DATE FROM BOOK_METADATA_AGGREGATION \
         ORDER BY SERIES_ID",
    )
    .fetch_all(pool)
    .await
    .expect("Read aggregation dates");
    rows.into_iter()
        .filter(|(id, created, modified)| {
            let previous = before
                .iter()
                .find(|(series, _)| series == id)
                .map_or(created, |(_, date)| date);
            modified != previous
        })
        .map(|(id, _, _)| id)
        .collect()
}

/// `(table, key)` of every row in the tables a scan fills.
pub async fn scan_rows(pool: &SqlitePool) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT 'SERIES', ID FROM SERIES UNION ALL SELECT 'BOOK', ID FROM BOOK \
         UNION ALL SELECT 'MEDIA', BOOK_ID FROM MEDIA \
         UNION ALL SELECT 'BOOK_METADATA', BOOK_ID FROM BOOK_METADATA \
         UNION ALL SELECT 'SERIES_METADATA', SERIES_ID FROM SERIES_METADATA \
         UNION ALL SELECT 'BOOK_METADATA_AGGREGATION', SERIES_ID FROM BOOK_METADATA_AGGREGATION \
         UNION ALL SELECT 'SIDECAR', URL FROM SIDECAR ORDER BY 1, 2",
    )
    .fetch_all(pool)
    .await
    .expect("Read content")
}
