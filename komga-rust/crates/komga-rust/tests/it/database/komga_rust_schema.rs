//! komga-rust's own migrations, recorded apart from Flyway's history.

use std::path::Path;

use komga_db::{Database, DbError};
use sqlx::SqlitePool;

use crate::support::db;

async fn migrate(path: &Path) -> Result<(), DbError> {
    let database = Database::connect(path).await.expect("Connect");
    let migrated = database.migrate().await;
    database.close().await;
    migrated
}

async fn schema_history(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar("SELECT VERSION FROM ERA_SCHEMA_HISTORY ORDER BY VERSION")
        .fetch_all(pool)
        .await
        .expect("ERA_SCHEMA_HISTORY")
}

#[tokio::test]
async fn databases_from_before_the_era_history_keep_their_era_tables() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let path = directory.path().join("database.sqlite");
    migrate(&path).await.expect("First migration");
    let pool = db::open(&path).await;
    assert_eq!(schema_history(&pool).await, ["0001", "0002", "0003"]);
    let flyway_komga_rust: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM flyway_schema_history WHERE script NOT LIKE 'V2%' AND type = 'SQL'",
    )
    .fetch_one(&pool)
    .await
    .expect("Flyway history");
    assert_eq!(
        flyway_komga_rust, 0,
        "Kotlin's history holds only its migrations"
    );
    // As left by builds that created the komga-rust tables at every startup.
    sqlx::raw_sql(
        r#"
        DROP TABLE ERA_SCHEMA_HISTORY;
        INSERT INTO ERA_SEARCH_INDEX_PENDING (ENTITY, ENTITY_ID) VALUES ('BOOK', 'b1');
        "#,
    )
    .execute(&pool)
    .await
    .expect("Pre-history database");
    pool.close().await;

    migrate(&path).await.expect("Adopting migration");
    let pool = db::open(&path).await;
    assert_eq!(schema_history(&pool).await, ["0001", "0002", "0003"]);
    let pending: Vec<String> = sqlx::query_scalar("SELECT ENTITY_ID FROM ERA_SEARCH_INDEX_PENDING")
        .fetch_all(&pool)
        .await
        .expect("Pending entities");
    assert_eq!(pending, ["b1"]);
    pool.close().await;
    let backups = std::fs::read_dir(directory.path().join("backups")).expect("Backups");
    assert_eq!(
        backups.count(),
        1,
        "pending komga-rust migrations back up first"
    );
}

#[tokio::test]
async fn era_history_from_a_newer_or_changed_build_is_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let path = directory.path().join("database.sqlite");
    migrate(&path).await.expect("First migration");
    let pool = db::open(&path).await;
    sqlx::query("INSERT INTO ERA_SCHEMA_HISTORY (VERSION, DESCRIPTION, CHECKSUM) VALUES ('9999', 'future', 0)")
        .execute(&pool)
        .await
        .expect("Future version");
    match migrate(&path).await {
        Err(DbError::UnknownVersion(version)) => assert_eq!(version, "era 9999"),
        other => panic!("Expected an unknown `era` version, got {other:?}"),
    }
    sqlx::query("DELETE FROM ERA_SCHEMA_HISTORY WHERE VERSION = '9999'")
        .execute(&pool)
        .await
        .expect("Remove future version");
    sqlx::query("UPDATE ERA_SCHEMA_HISTORY SET CHECKSUM = CHECKSUM + 1 WHERE VERSION = '0002'")
        .execute(&pool)
        .await
        .expect("Changed checksum");
    match migrate(&path).await {
        Err(DbError::ChecksumMismatch { version, .. }) => assert_eq!(version, "era 0002"),
        other => panic!("Expected an `era` checksum mismatch, got {other:?}"),
    }
    pool.close().await;
}
