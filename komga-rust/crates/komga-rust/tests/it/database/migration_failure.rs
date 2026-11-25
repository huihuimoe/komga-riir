//! A migration failing midway leaves the applied steps recorded, names the backup set, and the
//! set restores the data directory Kotlin left.

use std::fs;
use std::path::Path;

use komga_db::DbError;

use super::kotlin_versions::{fresh_komga_rust_directory, kotlin_data_directory, migrate, rows};
use crate::support::db;

/// A Kotlin schema with `book_projection` and `notable_entities` still pending.
const KOTLIN_VERSION: &str = "20260225161438";
const APPLIED: &str = "20260921111319";
const FAILING: &str = "20260929165528";

/// History versions and user tables of the main database, and history versions of the tasks one.
async fn state(directory: &Path) -> Vec<Vec<String>> {
    let main = db::main_pool(directory).await;
    let tasks = db::open(&directory.join("tasks.sqlite")).await;
    let state = vec![
        rows(
            &main,
            "SELECT version FROM flyway_schema_history ORDER BY installed_rank",
        )
        .await,
        rows(
            &main,
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
        )
        .await,
        rows(
            &tasks,
            "SELECT version FROM flyway_schema_history ORDER BY installed_rank",
        )
        .await,
    ];
    main.close().await;
    tasks.close().await;
    state
}

#[tokio::test]
async fn a_failed_migration_keeps_the_applied_steps_and_its_backup_set_restores_kotlin_data() {
    let fresh = fresh_komga_rust_directory().await;
    let dir = kotlin_data_directory(KOTLIN_VERSION, &fresh).await;
    // A table in the way of `notable_entities` makes that migration fail.
    let main = db::main_pool(dir.path()).await;
    sqlx::query("CREATE TABLE BOOK_METADATA_NOTABLE_ENTITIES (ID varchar)")
        .execute(&main)
        .await
        .expect("Block a migration");
    main.close().await;
    let before = state(dir.path()).await;

    let Err(DbError::MigrationAfterBackup { backup, .. }) = migrate(dir.path()).await else {
        panic!("The upgrade must fail after the backup");
    };

    let after = state(dir.path()).await;
    let history = &after[0];
    assert!(
        history.iter().any(|version| version == APPLIED),
        "{history:?}"
    );
    assert!(
        !history.iter().any(|version| version == FAILING),
        "{history:?}"
    );
    let tables = &after[1];
    assert!(tables.iter().any(|table| table == "BOOK_PROJECTION"));
    assert!(
        !tables
            .iter()
            .any(|table| table == "BOOK_METADATA_AGGREGATION_NOTABLE_ENTITIES")
    );

    for name in ["database.sqlite", "tasks.sqlite"] {
        fs::copy(backup.join(name), dir.path().join(name)).expect("Restore the database");
        for suffix in ["-wal", "-shm"] {
            let _ = fs::remove_file(dir.path().join(format!("{name}{suffix}")));
        }
    }
    assert_eq!(state(dir.path()).await, before);
}
