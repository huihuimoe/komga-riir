//! Rows the API cannot create yet, and data in the shapes earlier builds or Kotlin left. Seeded
//! rows skip what the services maintain, such as counts and metadata, unless a test sets it.

use sqlx::SqlitePool;

/// A library named `id` with the root `/{id}`.
pub async fn library(pool: &SqlitePool, id: &str) {
    sqlx::query("INSERT INTO LIBRARY (ID, NAME, ROOT) VALUES (?, ?, ?)")
        .bind(id)
        .bind(id)
        .bind(format!("/{id}"))
        .execute(pool)
        .await
        .expect("Failed to seed library");
}

/// A series named `id` at `/{id}`.
pub async fn series(pool: &SqlitePool, id: &str, library_id: &str) {
    sqlx::query(
        "INSERT INTO SERIES (ID, FILE_LAST_MODIFIED, NAME, URL, LIBRARY_ID) VALUES (?, CURRENT_TIMESTAMP, ?, ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(format!("/{id}"))
    .bind(library_id)
    .execute(pool)
    .await
    .expect("Failed to seed series");
}

/// A book named `id` at `/{id}`.
pub async fn book(pool: &SqlitePool, id: &str, series_id: &str, library_id: &str) {
    sqlx::query(
        "INSERT INTO BOOK (ID, FILE_LAST_MODIFIED, NAME, URL, SERIES_ID, LIBRARY_ID) VALUES (?, CURRENT_TIMESTAMP, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(id)
    .bind(format!("/{id}"))
    .bind(series_id)
    .bind(library_id)
    .execute(pool)
    .await
    .expect("Failed to seed book");
}

/// The metadata of a series titled `series_id`, with its age rating and sharing labels.
pub async fn series_metadata(
    pool: &SqlitePool,
    series_id: &str,
    age_rating: Option<i32>,
    labels: &[&str],
) {
    sqlx::query(
        "INSERT INTO SERIES_METADATA (SERIES_ID, STATUS, TITLE, TITLE_SORT, AGE_RATING) VALUES (?, 'ONGOING', ?, ?, ?)",
    )
    .bind(series_id)
    .bind(series_id)
    .bind(series_id)
    .bind(age_rating)
    .execute(pool)
    .await
    .expect("Failed to seed series metadata");
    for label in labels {
        sqlx::query("INSERT INTO SERIES_METADATA_SHARING (SERIES_ID, LABEL) VALUES (?, ?)")
            .bind(series_id)
            .bind(label)
            .execute(pool)
            .await
            .expect("Failed to seed sharing label");
    }
}

/// A collection named `id` holding `series_ids` in order.
pub async fn collection(pool: &SqlitePool, id: &str, series_ids: &[&str]) {
    sqlx::query("INSERT INTO COLLECTION (ID, NAME, SERIES_COUNT) VALUES (?, ?, ?)")
        .bind(id)
        .bind(id)
        .bind(i64::try_from(series_ids.len()).expect("Small collection"))
        .execute(pool)
        .await
        .expect("Failed to seed collection");
    for (number, series_id) in series_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO COLLECTION_SERIES (COLLECTION_ID, SERIES_ID, NUMBER) VALUES (?, ?, ?)",
        )
        .bind(id)
        .bind(series_id)
        .bind(i64::try_from(number).expect("Small collection index"))
        .execute(pool)
        .await
        .expect("Failed to seed collection member");
    }
}

/// A row in every table that references a user or its sync point, over a seeded book.
pub async fn user_owned_records(pool: &SqlitePool, user_id: &str) {
    library(pool, "test-library").await;
    series(pool, "test-series", "test-library").await;
    book(pool, "test-book", "test-series", "test-library").await;
    sqlx::query(
        "INSERT INTO READ_PROGRESS (BOOK_ID, USER_ID, PAGE, COMPLETED) VALUES (?, ?, ?, ?)",
    )
    .bind("test-book")
    .bind(user_id)
    .bind(1)
    .bind(false)
    .execute(pool)
    .await
    .expect("Failed to seed read progress");
    sqlx::query(
        "INSERT INTO READ_PROGRESS_SERIES (SERIES_ID, USER_ID, READ_COUNT, IN_PROGRESS_COUNT) VALUES (?, ?, ?, ?)",
    )
    .bind("test-series")
    .bind(user_id)
    .bind(0)
    .bind(1)
    .execute(pool)
    .await
    .expect("Failed to seed series read progress");
    user_relations(pool, user_id).await;
    sync_point(pool, user_id).await;
}

async fn user_relations(pool: &SqlitePool, user_id: &str) {
    sqlx::query("INSERT INTO USER_LIBRARY_SHARING (USER_ID, LIBRARY_ID) VALUES (?, ?)")
        .bind(user_id)
        .bind("test-library")
        .execute(pool)
        .await
        .expect("Failed to seed user library sharing");
    sqlx::query("INSERT INTO USER_SHARING (LABEL, ALLOW, USER_ID) VALUES (?, ?, ?)")
        .bind("restricted")
        .bind(true)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("Failed to seed user sharing");
    sqlx::query("INSERT INTO ANNOUNCEMENTS_READ (USER_ID, ANNOUNCEMENT_ID) VALUES (?, ?)")
        .bind(user_id)
        .bind("test-announcement")
        .execute(pool)
        .await
        .expect("Failed to seed read announcement");
    sqlx::query("INSERT INTO CLIENT_SETTINGS_USER (USER_ID, KEY, VALUE) VALUES (?, ?, ?)")
        .bind(user_id)
        .bind("test-setting")
        .bind("test-value")
        .execute(pool)
        .await
        .expect("Failed to seed client setting");
    sqlx::query("INSERT INTO USER_API_KEY (ID, USER_ID, API_KEY, COMMENT) VALUES (?, ?, ?, ?)")
        .bind("test-api-key")
        .bind(user_id)
        .bind("test-api-key-hash")
        .bind("test API key")
        .execute(pool)
        .await
        .expect("Failed to seed API key");
}

async fn sync_point(pool: &SqlitePool, user_id: &str) {
    sqlx::query("INSERT INTO SYNC_POINT (ID, USER_ID) VALUES (?, ?)")
        .bind("test-sync-point")
        .bind(user_id)
        .execute(pool)
        .await
        .expect("Failed to seed sync point");
    sqlx::query(
        "INSERT INTO SYNC_POINT_BOOK (SYNC_POINT_ID, BOOK_ID, BOOK_CREATED_DATE, BOOK_LAST_MODIFIED_DATE, BOOK_FILE_LAST_MODIFIED, BOOK_FILE_SIZE, BOOK_FILE_HASH, BOOK_METADATA_LAST_MODIFIED_DATE, SYNCED) VALUES (?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, 0, ?, CURRENT_TIMESTAMP, 0)",
    )
    .bind("test-sync-point")
    .bind("test-book")
    .bind("test-hash")
    .execute(pool)
    .await
    .expect("Failed to seed sync point book");
    sqlx::query(
        "INSERT INTO SYNC_POINT_BOOK_REMOVED_SYNCED (SYNC_POINT_ID, BOOK_ID) VALUES (?, ?)",
    )
    .bind("test-sync-point")
    .bind("removed-book")
    .execute(pool)
    .await
    .expect("Failed to seed removed sync point book");
    sqlx::query(
        "INSERT INTO SYNC_POINT_READLIST (SYNC_POINT_ID, READLIST_ID, READLIST_NAME, READLIST_CREATED_DATE, READLIST_LAST_MODIFIED_DATE) VALUES (?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
    )
    .bind("test-sync-point")
    .bind("test-readlist")
    .bind("Test read list")
    .execute(pool)
    .await
    .expect("Failed to seed sync point read list");
    sqlx::query(
        "INSERT INTO SYNC_POINT_READLIST_BOOK (SYNC_POINT_ID, READLIST_ID, BOOK_ID) VALUES (?, ?, ?)",
    )
    .bind("test-sync-point")
    .bind("test-readlist")
    .bind("test-book")
    .execute(pool)
    .await
    .expect("Failed to seed sync point read list book");
    sqlx::query(
        "INSERT INTO SYNC_POINT_READLIST_REMOVED_SYNCED (SYNC_POINT_ID, READLIST_ID) VALUES (?, ?)",
    )
    .bind("test-sync-point")
    .bind("removed-readlist")
    .execute(pool)
    .await
    .expect("Failed to seed removed sync point read list");
}

/// Sets, or with `None` removes, server settings.
pub async fn server_settings(pool: &SqlitePool, settings: &[(&str, Option<&str>)]) {
    for (key, value) in settings {
        let query = match value {
            Some(value) => sqlx::query(
                "INSERT INTO SERVER_SETTINGS (KEY, VALUE) VALUES (?, ?) \
                 ON CONFLICT (KEY) DO UPDATE SET VALUE = excluded.VALUE",
            )
            .bind(key)
            .bind(value),
            None => sqlx::query("DELETE FROM SERVER_SETTINGS WHERE KEY = ?").bind(key),
        };
        query
            .execute(pool)
            .await
            .expect("Failed to apply settings fixture");
    }
}

/// Overwrites the root of every library.
pub async fn library_root(pool: &SqlitePool, root: &str) {
    sqlx::query("UPDATE LIBRARY SET ROOT = ?")
        .bind(root)
        .execute(pool)
        .await
        .expect("Legacy root");
}

/// Rates a scanned series as a metadata edit would.
pub async fn series_age_rating(pool: &SqlitePool, series_id: &str, age_rating: i32) {
    sqlx::query("UPDATE SERIES_METADATA SET AGE_RATING = ? WHERE SERIES_ID = ?")
        .bind(age_rating)
        .bind(series_id)
        .execute(pool)
        .await
        .expect("Failed to rate series");
}

/// Sets a media status as a scan of a changed file leaves it before the analysis runs.
pub async fn media_status(pool: &SqlitePool, book_id: &str, status: &str) {
    sqlx::query("UPDATE MEDIA SET STATUS = ? WHERE BOOK_ID = ?")
        .bind(status)
        .bind(book_id)
        .execute(pool)
        .await
        .expect("Failed to set media status");
}

/// Book metadata as metadata providers or edits would set it.
pub async fn book_metadata(
    pool: &SqlitePool,
    book_id: &str,
    (summary, isbn, release_date): (&str, &str, &str),
    authors: &[(&str, &str)],
    tags: &[&str],
) {
    sqlx::query(
        "UPDATE BOOK_METADATA SET SUMMARY = ?, ISBN = ?, RELEASE_DATE = ? WHERE BOOK_ID = ?",
    )
    .bind(summary)
    .bind(isbn)
    .bind(release_date)
    .bind(book_id)
    .execute(pool)
    .await
    .expect("Failed to seed book metadata");
    for (name, role) in authors {
        sqlx::query("INSERT INTO BOOK_METADATA_AUTHOR (NAME, ROLE, BOOK_ID) VALUES (?, ?, ?)")
            .bind(name)
            .bind(role)
            .bind(book_id)
            .execute(pool)
            .await
            .expect("Failed to seed author");
    }
    for tag in tags {
        sqlx::query("INSERT INTO BOOK_METADATA_TAG (TAG, BOOK_ID) VALUES (?, ?)")
            .bind(tag)
            .bind(book_id)
            .execute(pool)
            .await
            .expect("Failed to seed tag");
    }
}

/// The language and reading direction of a scanned series, as metadata edits would set them.
pub async fn series_reading(pool: &SqlitePool, series_id: &str, language: &str, direction: &str) {
    sqlx::query(
        "UPDATE SERIES_METADATA SET LANGUAGE = ?, READING_DIRECTION = ? WHERE SERIES_ID = ?",
    )
    .bind(language)
    .bind(direction)
    .bind(series_id)
    .execute(pool)
    .await
    .expect("Failed to seed series reading");
}
