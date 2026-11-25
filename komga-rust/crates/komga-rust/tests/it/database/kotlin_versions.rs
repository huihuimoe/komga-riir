//! Upgrades from every supported Kotlin schema version. Each data directory replays the Kotlin
//! SQL up to that version, with the Flyway history Kotlin records, and holds the data Kotlin had
//! written by then: users with their roles and credentials, content, reading state, assets,
//! sync anchors and queued tasks.

use std::fs;
use std::path::{Path, PathBuf};

use komga_db::{Database, DbError};
use komga_test_support::TestDir;
use reqwest::{Method, StatusCode};
use sqlx::SqlitePool;

use crate::support::{ADMIN_EMAIL, ADMIN_PASSWORD, TestServer, db, tasks, template};

const FLOOR: &str = "20240422132621";
const KOTLIN_TASKS_INDICES_MAIN: &str = "20261002174155";
const APIKEY: &str = "20240529120933";
const RECOMPUTE_KOREADER_HASH: &str = "20261020111142";
const READER_EMAIL: &str = "reader@example.com";
const API_KEY: &str = "kotlin-api-key";
const THUMBNAIL: &[u8] = b"\xff\xd8\xff\xd9";

/// The Kotlin SQL migrations of `directory` (`main` or `tasks`), as `(version, file name, SQL)` in
/// order, with the placeholder values Kotlin configures.
pub(super) fn kotlin_sql_migrations(directory: &str) -> Vec<(String, String, String)> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../komga-db/migrations")
        .join(directory);
    let mut files: Vec<_> = fs::read_dir(directory)
        .expect("Kotlin migrations")
        .map(|entry| entry.expect("Migration entry").path())
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|file| {
            let name = file
                .file_name()
                .and_then(|name| name.to_str())
                .expect("Name");
            let version = name[1..name.find("__").expect("Flyway name")].to_owned();
            let name = name.to_owned();
            let sql = fs::read_to_string(&file)
                .expect("Migration SQL")
                .replace("${delete-empty-collections}", "1")
                .replace("${delete-empty-read-lists}", "1")
                .replace("${library-file-hashing}", "1")
                .replace("${library-scan-startup}", "0");
            (version, name, sql)
        })
        .collect()
}

/// Applies the Kotlin migrations of `directory` in `(after, through]` to `pool`, copying their
/// Flyway history rows from the fresh database `fresh`. JDBC migrations only fix existing data,
/// which never needs fixing here, so only their history rows are copied.
async fn replay(
    pool: &SqlitePool,
    fresh: &SqlitePool,
    directory: &str,
    after: &str,
    through: &str,
) {
    if after.is_empty() {
        sqlx::raw_sql(super::migrations::CREATE_FLYWAY_SCHEMA_HISTORY)
            .execute(pool)
            .await
            .expect("Create the Flyway history");
    }
    for (version, _, sql) in kotlin_sql_migrations(directory) {
        if version.as_str() > after && version.as_str() <= through {
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(pool)
                .await
                .expect("Apply a Kotlin migration");
        }
    }
    let rows: Vec<(i64, String, String, String, String, Option<i32>)> = sqlx::query_as(
        "SELECT installed_rank, version, description, type, script, checksum FROM flyway_schema_history WHERE version > ? AND version <= ? ORDER BY installed_rank",
    )
    .bind(after)
    .bind(through)
    .fetch_all(fresh)
    .await
    .expect("Read the fresh history");
    let mut tx = pool.begin().await.expect("Begin history insert");
    for (rank, version, description, kind, script, checksum) in rows {
        sqlx::query("INSERT INTO flyway_schema_history (installed_rank, version, description, type, script, checksum, installed_by, execution_time, success) VALUES (?, ?, ?, ?, ?, ?, 'komga', 1, 1)")
            .bind(rank)
            .bind(version)
            .bind(description)
            .bind(kind)
            .bind(script)
            .bind(checksum)
            .execute(&mut *tx)
            .await
            .expect("Record a Kotlin migration");
    }
    tx.commit().await.expect("Commit history insert");
}

/// What Kotlin had written by the floor, in the floor schema: roles are still `USER` columns.
async fn seed_floor(pool: &SqlitePool) {
    let password = "$2a$10$Sn4uwlkCYlFJgoS2FGDNT.DcfvZ3NCROP7uRJLCoqP0O0jVVru6Mq";
    sqlx::query(
        r#"
        INSERT INTO USER (ID, EMAIL, PASSWORD, ROLE_ADMIN, ROLE_FILE_DOWNLOAD, ROLE_PAGE_STREAMING)
            VALUES ('A', ?, ?, 1, 1, 1), ('R', ?, ?, 0, 1, 0)
        "#,
    )
    .bind(ADMIN_EMAIL)
    .bind(password)
    .bind(READER_EMAIL)
    .bind(password)
    .execute(pool)
    .await
    .expect("Seed users");
    sqlx::raw_sql(r#"
        INSERT INTO LIBRARY (ID, NAME, ROOT) VALUES ('L', 'Library', 'file:/data/Library');
        INSERT INTO SERIES (ID, NAME, URL, LIBRARY_ID, FILE_LAST_MODIFIED)
            VALUES ('S', 'Series', 'file:/data/Library/Series', 'L', '2024-01-01 00:00:00');
        INSERT INTO SERIES_METADATA (SERIES_ID, STATUS, TITLE, TITLE_SORT)
            VALUES ('S', 'ONGOING', 'Series', 'Series');
        INSERT INTO BOOK (ID, NAME, URL, SERIES_ID, LIBRARY_ID, FILE_LAST_MODIFIED)
            VALUES ('B', 'Book', 'file:/data/Library/Series/Book.cbz', 'S', 'L', '2024-01-01 00:00:00');
        INSERT INTO MEDIA (BOOK_ID, STATUS, MEDIA_TYPE, PAGE_COUNT)
            VALUES ('B', 'READY', 'application/zip', 1);
        INSERT INTO MEDIA_PAGE (BOOK_ID, FILE_NAME, MEDIA_TYPE, NUMBER)
            VALUES ('B', '1.jpg', 'image/jpeg', 0);
        INSERT INTO BOOK_METADATA (BOOK_ID, NUMBER, NUMBER_SORT, TITLE) VALUES ('B', '1', 1, 'Book');
        INSERT INTO READ_PROGRESS (BOOK_ID, USER_ID, PAGE, COMPLETED, READ_DATE)
            VALUES ('B', 'A', 1, 1, '2024-01-02 03:04:05');
        INSERT INTO READ_PROGRESS_SERIES (SERIES_ID, USER_ID, READ_COUNT, IN_PROGRESS_COUNT)
            VALUES ('S', 'A', 1, 0);
        INSERT INTO THUMBNAIL_BOOK (ID, THUMBNAIL, SELECTED, TYPE, BOOK_ID, MEDIA_TYPE, FILE_SIZE, WIDTH, HEIGHT)
            VALUES ('T', x'ffd8ffd9', 1, 'GENERATED', 'B', 'image/jpeg', 4, 1, 1);
        INSERT INTO READLIST (ID, NAME, BOOK_COUNT) VALUES ('RL', 'Reading', 1);
        INSERT INTO READLIST_BOOK (READLIST_ID, BOOK_ID, NUMBER) VALUES ('RL', 'B', 0);
        "#)
        .execute(pool)
        .await
        .expect("Seed content");
}

/// What Kotlin at `version` writes into the tables its later migrations created.
async fn seed_later(pool: &SqlitePool, version: &str) {
    let seeds = [
        // SHA-512 of `API_KEY`, as Kotlin's `TokenEncoder` stores it.
        (
            APIKEY,
            r#"INSERT INTO USER_API_KEY (ID, USER_ID, API_KEY, COMMENT) VALUES ('K', 'A', '46cefe446a179c10a003578f12bc889a2e5c821659ebe2e82a0985cd7895da2fafd6594f4fbef41d2bb0af4a21420b3ac506e65f998d8b69e6e865afe646eca6', 'Kobo')"#,
        ),
        (
            "20240529120934",
            r#"
            INSERT INTO SYNC_POINT (ID, USER_ID, API_KEY_ID) VALUES ('P', 'A', 'K');
            INSERT INTO SYNC_POINT_BOOK (SYNC_POINT_ID, BOOK_ID, BOOK_CREATED_DATE, BOOK_LAST_MODIFIED_DATE, BOOK_FILE_LAST_MODIFIED, BOOK_FILE_SIZE, BOOK_FILE_HASH, BOOK_METADATA_LAST_MODIFIED_DATE)
                VALUES ('P', 'B', '2024-01-01 00:00:00', '2024-01-01 00:00:00', '2024-01-01 00:00:00', 1, 'hash', '2024-01-01 00:00:00')
            "#,
        ),
        (
            "20240909112827",
            r#"INSERT INTO SYNC_POINT_READLIST (SYNC_POINT_ID, READLIST_ID, READLIST_NAME, READLIST_CREATED_DATE, READLIST_LAST_MODIFIED_DATE) VALUES ('P', 'RL', 'Reading', '2024-01-01 00:00:00', '2024-01-01 00:00:00')"#,
        ),
        (
            "20250108172343",
            "UPDATE BOOK SET FILE_HASH_KOREADER = 'koreader'",
        ),
        (
            "20250205151235",
            r#"INSERT INTO CLIENT_SETTINGS_USER (USER_ID, KEY, VALUE) VALUES ('A', 'webui.theme', '"dark"')"#,
        ),
    ];
    for (since, sql) in seeds {
        if since <= version {
            sqlx::raw_sql(sql)
                .execute(pool)
                .await
                .expect("Seed later data");
        }
    }
}

/// Shared read-only fresh komga-rust data directory (migrated once per migration fingerprint and
/// checkpointed back to `DELETE` journal mode so concurrent readers never create WAL/SHM files).
pub(super) async fn fresh_komga_rust_directory() -> PathBuf {
    let key = format!(
        "fresh-komga-rust-{:08x}",
        template::migrations_fingerprint()
    );
    template::cached_dir(&key, |dir| async move {
        migrate(&dir).await.expect("Migrate a fresh directory");
        for name in ["database.sqlite", "tasks.sqlite"] {
            let pool = db::open(&dir.join(name)).await;
            sqlx::raw_sql("PRAGMA journal_mode = DELETE;")
                .execute(&pool)
                .await
                .expect("Checkpoint fresh template");
            pool.close().await;
        }
    })
    .await
}

/// A data directory Kotlin Komga at `version` left, beside a fresh komga-rust one.
pub(super) async fn kotlin_data_directory(version: &str, fresh: &Path) -> TestDir {
    let fresh_main = db::open_read_only(&fresh.join("database.sqlite")).await;
    let fresh_tasks = db::open_read_only(&fresh.join("tasks.sqlite")).await;
    let dir = komga_test_support::tempdir().expect("Data directory");
    let main = db::main_pool(dir.path()).await;
    db::disable_sync(&main).await;
    replay(&main, &fresh_main, "main", "", FLOOR).await;
    seed_floor(&main).await;
    replay(&main, &fresh_main, "main", FLOOR, version).await;
    seed_later(&main, version).await;
    main.close().await;

    // The tasks migration released together with the main one.
    let tasks_version = if version >= KOTLIN_TASKS_INDICES_MAIN {
        "20261002175358"
    } else {
        "20231013114850"
    };
    let pool = db::open(&dir.path().join("tasks.sqlite")).await;
    db::disable_sync(&pool).await;
    replay(&pool, &fresh_tasks, "tasks", "", tasks_version).await;
    pool.close().await;
    fresh_main.close().await;
    fresh_tasks.close().await;
    tasks::queue_kotlin_analysis(dir.path(), "B", "S").await;
    tasks::queue_kotlin_removed_task(dir.path()).await;
    dir
}

pub(super) async fn migrate(dir: &Path) -> Result<(), DbError> {
    let main = Database::connect(&dir.join("database.sqlite")).await?;
    let tasks = Database::connect(&dir.join("tasks.sqlite")).await?;
    let migrated = Database::migrate_all(&main, &tasks, &dir.join("backups")).await;
    main.close().await;
    tasks.close().await;
    migrated
}

pub(super) async fn rows(pool: &SqlitePool, query: &str) -> Vec<String> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(query))
        .fetch_all(pool)
        .await
        .expect("Query")
}

/// The schema, Flyway history and `ERA_SCHEMA_HISTORY`, which must match a fresh komga-rust database.
async fn schema(directory: &Path) -> Vec<Vec<String>> {
    let pool = db::open_read_only(&directory.join("database.sqlite")).await;
    let tasks = db::open_read_only(&directory.join("tasks.sqlite")).await;
    let schema = vec![
        rows(&pool, "SELECT type || ' ' || name || ' ' || coalesce(sql, '') FROM sqlite_master WHERE name <> 'flyway_schema_history' ORDER BY name").await,
        rows(&pool, "SELECT installed_rank || ' ' || version || ' ' || description || ' ' || type || ' ' || script || ' ' || coalesce(checksum, '') || ' ' || success FROM flyway_schema_history ORDER BY installed_rank").await,
        rows(&pool, "SELECT VERSION || ' ' || CHECKSUM FROM ERA_SCHEMA_HISTORY ORDER BY VERSION").await,
        rows(&tasks, "SELECT type || ' ' || name || ' ' || coalesce(sql, '') FROM sqlite_master WHERE name <> 'flyway_schema_history' ORDER BY name").await,
        rows(&tasks, "SELECT installed_rank || ' ' || version || ' ' || coalesce(checksum, '') FROM flyway_schema_history ORDER BY installed_rank").await,
    ];
    pool.close().await;
    tasks.close().await;
    schema
}

/// Row counts of every table, which no migration step may change.
async fn row_counts(pool: &SqlitePool) -> Vec<String> {
    let mut counts = Vec::new();
    for table in rows(pool, "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT IN ('flyway_schema_history', 'TEMP_STRING_LIST') ORDER BY name").await {
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM \"{table}\"")))
            .fetch_one(pool)
            .await
            .expect("Count rows");
        counts.push(format!("{table} {count}"));
    }
    counts
}

/// What the Kotlin data migrations derive, whether Kotlin or rust applied them.
async fn assert_derived_data(pool: &SqlitePool, version: &str) {
    let roles = rows(
        pool,
        "SELECT USER_ID || ' ' || ROLE FROM USER_ROLE ORDER BY USER_ID, ROLE",
    )
    .await;
    assert_eq!(
        roles,
        [
            "A ADMIN",
            "A FILE_DOWNLOAD",
            "A KOBO_SYNC",
            "A KOREADER_SYNC",
            "A PAGE_STREAMING",
            "R FILE_DOWNLOAD"
        ]
    );
    let read_date = rows(
        pool,
        "SELECT MOST_RECENT_READ_DATE FROM READ_PROGRESS_SERIES",
    )
    .await;
    assert_eq!(read_date, ["2024-01-02 03:04:05"]);
    // recomputes the hashes only when Kotlin has not yet.
    let hash = rows(pool, "SELECT FILE_HASH_KOREADER FROM BOOK").await;
    let expected = if version >= RECOMPUTE_KOREADER_HASH {
        "koreader"
    } else {
        ""
    };
    assert_eq!(hash, [expected]);
}

/// Logs in with the passwords and, once Kotlin had API keys, the API key; serves the stored
/// thumbnail.
async fn assert_served(directory: &Path, version: &str) {
    let server = TestServer::start(directory).await;
    for (email, password) in [
        (ADMIN_EMAIL, ADMIN_PASSWORD),
        (READER_EMAIL, ADMIN_PASSWORD),
    ] {
        let response = server
            .request(Method::GET, "/api/v2/users/me")
            .basic_auth(email, Some(password))
            .send()
            .await
            .expect("Password login");
        assert_eq!(response.status(), StatusCode::OK, "{email}");
    }
    let request = server.request(Method::GET, "/api/v1/books/B/thumbnail");
    let request = if version >= APIKEY {
        request.header("X-API-Key", API_KEY)
    } else {
        request.basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
    };
    let response = request.send().await.expect("API key request");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.bytes().await.expect("Thumbnail").as_ref(),
        THUMBNAIL
    );
    server.stop().await;
}

async fn upgrade_from(version: &str) {
    let fresh = fresh_komga_rust_directory().await;
    let dir = kotlin_data_directory(version, &fresh).await;
    let main = db::main_pool(dir.path()).await;
    let counts = row_counts(&main).await;
    main.close().await;
    let queued = tasks::tasks(dir.path()).await;

    migrate(dir.path()).await.expect("Upgrade from Kotlin");

    assert_eq!(schema(dir.path()).await, schema(&fresh).await);
    let main = db::main_pool(dir.path()).await;
    let migrated = row_counts(&main).await;
    let kept: Vec<_> = migrated
        .iter()
        .filter(|row| {
            counts
                .iter()
                .any(|count| count.split(' ').next() == row.split(' ').next())
        })
        .collect();
    assert_eq!(kept, counts.iter().collect::<Vec<_>>());
    assert_derived_data(&main, version).await;
    main.close().await;
    assert_eq!(tasks::tasks(dir.path()).await, queued);
    assert_served(dir.path(), version).await;
}

macro_rules! from_kotlin {
    ($($name:ident: $version:literal,)*) => {
        const VERSIONS: &[&str] = &[$($version),*];
        $(
            #[tokio::test]
            async fn $name() {
                upgrade_from($version).await;
            }
        )*
    };
}

from_kotlin! {
    from_20240422132621_fix_read_progress_locators: "20240422132621",
    from_20240529120933_apikey: "20240529120933",
    from_20240529120934_syncpoint: "20240529120934",
    from_20240614170012_user_kobo_role: "20240614170012",
    from_20240906152500_read_progress_series_read_date: "20240906152500",
    from_20240909112827_syncpoint_readlists: "20240909112827",
    from_20240911175419_kepub: "20240911175419",
    from_20240925174446_syncpoint_covers: "20240925174446",
    from_20250108115503_user_roles: "20250108115503",
    from_20250108172343_koreader_hash: "20250108172343",
    from_20250205151235_client_settings: "20250205151235",
    from_20250730173126_remove_temp_string_table: "20250730173126",
    from_20260225161438_tags_view: "20260225161438",
    from_20260921111319_book_projection: "20260921111319",
    from_20260929165528_notable_entities: "20260929165528",
    from_20261002174155_missing_indices: "20261002174155",
    from_20261020111142_recompute_koreader_hash: "20261020111142",
}

#[test]
fn every_main_migration_from_the_floor_is_a_starting_version() {
    let versions: Vec<_> = kotlin_sql_migrations("main")
        .into_iter()
        .map(|(version, _, _)| version)
        .filter(|version| version.as_str() > FLOOR)
        .collect();
    assert_eq!(VERSIONS[0], FLOOR);
    assert_eq!(VERSIONS[1..], versions);
}
