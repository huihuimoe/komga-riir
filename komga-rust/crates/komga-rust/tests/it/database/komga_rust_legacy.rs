//! Adoption of databases migrated by komga-rust; delete with `migration/komga_rust_legacy.rs`.

use std::fs;
use std::path::MAIN_SEPARATOR;

use komga_db::DbError;
use komga_test_support::TestDir;
use sqlx::SqlitePool;

use super::kotlin_versions::{kotlin_sql_migrations, migrate};
use crate::support::db;

/// The first Kotlin migration komga-rust does not ship.
const FIRST_MAIN_MIGRATION_AFTER_KOMGA_RUST: &str = "20260921111319";
#[cfg(unix)]
const ROOT_PATH: &str = "/data/My Library";
#[cfg(windows)]
const ROOT_PATH: &str = r"C:\data\My Library";
#[cfg(unix)]
const ROOT_URL: &str = "file:/data/My%20Library";
#[cfg(windows)]
const ROOT_URL: &str = "file:/C:/data/My%20Library";
const CREATE_SQLX_MIGRATIONS: &str = r#"
    CREATE TABLE _sqlx_migrations (
        version BIGINT PRIMARY KEY,
        description TEXT NOT NULL,
        installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        success BOOLEAN NOT NULL,
        checksum BLOB NOT NULL,
        execution_time BIGINT NOT NULL
    )
"#;

/// Applies the Kotlin SQL migrations of `directory` older than `before` as komga-rust does,
/// recording them in `_sqlx_migrations` only.
async fn migrate_as_komga_rust(pool: &SqlitePool, directory: &str, before: &str) {
    db::disable_sync(pool).await;
    sqlx::raw_sql(CREATE_SQLX_MIGRATIONS)
        .execute(pool)
        .await
        .expect("Create the SQLx history");
    for (version, name, sql) in kotlin_sql_migrations(directory) {
        if version.as_str() >= before {
            break;
        }
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await
            .expect("Apply a Kotlin migration");
        sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (?, ?, 1, x'00', 0)")
            .bind(version.parse::<i64>().expect("Numeric version"))
            .bind(name)
            .execute(pool)
            .await
            .expect("Record the migration");
    }
}

/// A data directory komga-rust created, with one library scanned into it.
async fn komga_rust_data_directory() -> TestDir {
    let dir = komga_test_support::tempdir().expect("Data directory");
    let main = db::main_pool(dir.path()).await;
    migrate_as_komga_rust(&main, "main", FIRST_MAIN_MIGRATION_AFTER_KOMGA_RUST).await;
    let seed = format!(
        "
        INSERT INTO LIBRARY (ID, NAME, ROOT) VALUES ('L', 'Library', '{ROOT_PATH}');
        INSERT INTO SERIES (ID, NAME, URL, LIBRARY_ID, FILE_LAST_MODIFIED)
            VALUES ('S', 'Series', '{ROOT_PATH}{MAIN_SEPARATOR}Series', 'L', CURRENT_TIMESTAMP);
        INSERT INTO SERIES (ID, NAME, URL, LIBRARY_ID, FILE_LAST_MODIFIED, ONESHOT)
            VALUES ('O', 'Oneshot', '{ROOT_PATH}{MAIN_SEPARATOR}_oneshots{MAIN_SEPARATOR}One.cbz', 'L', CURRENT_TIMESTAMP, 1);
        INSERT INTO BOOK (ID, NAME, URL, SERIES_ID, LIBRARY_ID, FILE_LAST_MODIFIED)
            VALUES ('B', 'Book', '{ROOT_PATH}{MAIN_SEPARATOR}Series{MAIN_SEPARATOR}Book #1.cbz', 'S', 'L', CURRENT_TIMESTAMP);
        INSERT INTO BOOK (ID, NAME, URL, SERIES_ID, LIBRARY_ID, FILE_LAST_MODIFIED)
            VALUES ('K', 'Kept', '{ROOT_URL}/Series/Kept.cbz', 'S', 'L', CURRENT_TIMESTAMP);
        INSERT INTO SIDECAR (URL, PARENT_URL, LAST_MODIFIED_TIME, LIBRARY_ID) VALUES
            ('{ROOT_PATH}{MAIN_SEPARATOR}Series{MAIN_SEPARATOR}cover.jpg', '{ROOT_PATH}{MAIN_SEPARATOR}Series', CURRENT_TIMESTAMP, 'L'),
            ('{ROOT_PATH}{MAIN_SEPARATOR}Series{MAIN_SEPARATOR}Book #1.jpg', '{ROOT_PATH}{MAIN_SEPARATOR}Series{MAIN_SEPARATOR}Book #1.cbz', CURRENT_TIMESTAMP, 'L');
        INSERT INTO USER (ID, EMAIL, PASSWORD) VALUES ('U', 'u@x', 'p');
        INSERT INTO READ_PROGRESS (BOOK_ID, USER_ID, PAGE, COMPLETED, READ_DATE, CREATED_DATE)
            VALUES ('B', 'U', 1, 0, '2026-09-15T17:57:23.737Z', '2026-09-15T09:00:00+09:00');
        "
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(seed))
        .execute(&main)
        .await
        .expect("Seed the library");
    main.close().await;

    let tasks = db::open(&dir.path().join("tasks.sqlite")).await;
    migrate_as_komga_rust(&tasks, "tasks", "99999999999999").await;
    tasks.close().await;
    dir
}

async fn strings(pool: &SqlitePool, query: &str) -> Vec<String> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(query))
        .fetch_all(pool)
        .await
        .expect("Query")
}

#[tokio::test]
async fn a_komga_rust_database_is_adopted_with_flyway_history_file_urls_and_datetimes() {
    let dir = komga_rust_data_directory().await;
    migrate(dir.path())
        .await
        .expect("Adopt komga-rust databases");

    let main = db::main_pool(dir.path()).await;
    let jdbc = strings(
        &main,
        "SELECT installed_by FROM flyway_schema_history WHERE type = 'JDBC'",
    )
    .await;
    assert_eq!(jdbc, ["komga-rust"; 5]);
    let missing_or_failed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM flyway_schema_history WHERE success = 0 OR checksum IS NULL AND type = 'SQL'",
    )
    .fetch_one(&main)
    .await
    .expect("History");
    assert_eq!(missing_or_failed, 0);
    assert!(
        strings(
            &main,
            "SELECT name FROM sqlite_master WHERE name = '_sqlx_migrations'"
        )
        .await
        .is_empty()
    );
    assert_eq!(
        strings(&main, "SELECT ROOT FROM LIBRARY").await,
        [format!("{ROOT_URL}/")]
    );
    assert_eq!(
        strings(&main, "SELECT URL FROM SERIES ORDER BY ID").await,
        [
            format!("{ROOT_URL}/_oneshots/One.cbz"),
            format!("{ROOT_URL}/Series/"),
        ]
    );
    assert_eq!(
        strings(&main, "SELECT URL FROM BOOK ORDER BY ID").await,
        [
            format!("{ROOT_URL}/Series/Book%20%231.cbz"),
            format!("{ROOT_URL}/Series/Kept.cbz"),
        ]
    );
    assert_eq!(
        strings(
            &main,
            "SELECT URL || ' ' || PARENT_URL FROM SIDECAR ORDER BY URL"
        )
        .await,
        [
            format!("{ROOT_URL}/Series/Book%20%231.jpg {ROOT_URL}/Series/Book%20%231.cbz"),
            format!("{ROOT_URL}/Series/cover.jpg {ROOT_URL}/Series/"),
        ]
    );
    assert_eq!(
        strings(
            &main,
            "SELECT READ_DATE || ' ' || CREATED_DATE FROM READ_PROGRESS"
        )
        .await,
        ["2026-09-15 17:57:23.737 2026-09-15 00:00:00.000"]
    );
    main.close().await;

    let tasks = db::open(&dir.path().join("tasks.sqlite")).await;
    assert_eq!(
        strings(&tasks, "SELECT version FROM flyway_schema_history")
            .await
            .len(),
        2
    );
    tasks.close().await;

    let backups = fs::read_dir(dir.path().join("backups"))
        .expect("Backups")
        .count();
    assert_eq!(backups, 1);
    migrate(dir.path())
        .await
        .expect("Adopted databases open as Komga ones");
    assert_eq!(
        fs::read_dir(dir.path().join("backups"))
            .expect("Backups")
            .count(),
        1
    );
}

#[tokio::test]
async fn a_komga_rust_history_with_an_unknown_version_is_refused_before_writing() {
    let dir = komga_rust_data_directory().await;
    let tasks = db::open(&dir.path().join("tasks.sqlite")).await;
    sqlx::query("INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (1, 'unknown', 1, x'00', 0)")
        .execute(&tasks)
        .await
        .expect("Record an unknown migration");
    tasks.close().await;

    let error = migrate(dir.path())
        .await
        .expect_err("Unknown komga-rust migrations are refused");
    assert!(
        matches!(&error, DbError::UnknownVersion(version) if version == "komga-rust 1"),
        "{error}"
    );
    assert!(!dir.path().join("backups").exists());
    let main = db::main_pool(dir.path()).await;
    assert_eq!(
        strings(&main, "SELECT ROOT FROM LIBRARY").await,
        [ROOT_PATH]
    );
    main.close().await;
}
