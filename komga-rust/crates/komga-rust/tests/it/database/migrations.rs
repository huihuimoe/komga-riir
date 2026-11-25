use komga_db::{Database, DbError, DirectoryLock};
use sqlx::SqlitePool;

use crate::support::db;

const EXPECTED_JDBC_MIGRATIONS: [(&str, &str, &str); 5] = [
    (
        "20200810154730",
        "thumbnails part 2",
        "db.migration.sqlite.V20200810154730__thumbnails_part_2",
    ),
    (
        "20200820150923",
        "metadata fields part 2",
        "db.migration.sqlite.V20200820150923__metadata_fields_part_2",
    ),
    (
        "20210624165023",
        "missing series metadata",
        "db.migration.sqlite.V20210624165023__missing_series_metadata",
    ),
    (
        "20230801104436",
        "fix incorrect language codes",
        "db.migration.sqlite.V20230801104436__fix_incorrect_language_codes",
    ),
    (
        "20240422132621",
        "fix read progress locators",
        "db.migration.sqlite.V20240422132621__fix_read_progress_locators",
    ),
];

pub(super) const CREATE_FLYWAY_SCHEMA_HISTORY: &str = r#"
    CREATE TABLE flyway_schema_history (
        installed_rank INTEGER NOT NULL PRIMARY KEY,
        version VARCHAR(50),
        description VARCHAR(200) NOT NULL,
        type VARCHAR(20) NOT NULL,
        script VARCHAR(1000) NOT NULL,
        checksum INTEGER,
        installed_by VARCHAR(100) NOT NULL,
        installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
        execution_time INTEGER NOT NULL,
        success BOOLEAN NOT NULL
    )
"#;

async fn create_flyway_schema_history(pool: &SqlitePool) {
    sqlx::query(CREATE_FLYWAY_SCHEMA_HISTORY)
        .execute(pool)
        .await
        .expect("Failed to create Flyway history table");
}

async fn seed_jdbc_history(pool: &SqlitePool) {
    for (index, (version, description, script)) in EXPECTED_JDBC_MIGRATIONS.iter().enumerate() {
        sqlx::query(
            "INSERT INTO flyway_schema_history (installed_rank, version, description, type, script, installed_by, execution_time, success) VALUES (?, ?, ?, 'JDBC', ?, 'komga', 0, 1)",
        )
        .bind(i64::try_from(index + 1).expect("JDBC migration rank fits in i64"))
        .bind(*version)
        .bind(*description)
        .bind(*script)
        .execute(pool)
        .await
        .expect("Failed to seed JDBC migration history");
    }
}

#[tokio::test]
async fn test_empty_database_migration_and_persistence() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let db_path = temp_dir.path().join("database.sqlite");

    // 1. Initial migration on empty database
    {
        let db = Database::connect(&db_path)
            .await
            .expect("Failed to connect to SQLite");
        db.migrate().await.expect("Failed to run initial migration");
        let pool = db::open(&db_path).await;

        // Insert a test record in USER and USER_ROLE
        sqlx::query("INSERT INTO USER (ID, EMAIL, PASSWORD) VALUES ($1, $2, $3)")
            .bind("user-1")
            .bind("admin@example.com")
            .bind("hashed_secret")
            .execute(&pool)
            .await
            .expect("Failed to insert user");

        sqlx::query("INSERT INTO USER_ROLE (USER_ID, ROLE) VALUES ($1, $2)")
            .bind("user-1")
            .bind("ADMIN")
            .execute(&pool)
            .await
            .expect("Failed to insert user role");
    }

    // 2. Re-open and verify persistence
    {
        let db = Database::connect(&db_path)
            .await
            .expect("Failed to re-connect to SQLite");
        db.migrate()
            .await
            .expect("Re-migration on existing DB must succeed");
        let pool = db::open(&db_path).await;

        let email: String = sqlx::query_scalar("SELECT EMAIL FROM USER WHERE ID = 'user-1'")
            .fetch_one(&pool)
            .await
            .expect("Failed to query user");
        assert_eq!(email, "admin@example.com");

        let jdbc_migrations: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT version, description, script FROM flyway_schema_history WHERE type = 'JDBC' ORDER BY installed_rank",
        )
        .fetch_all(&pool)
        .await
        .expect("Failed to read JDBC migration history");
        let expected_jdbc_migrations = EXPECTED_JDBC_MIGRATIONS
            .iter()
            .map(|(version, description, script)| {
                (
                    (*version).to_owned(),
                    (*description).to_owned(),
                    (*script).to_owned(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(jdbc_migrations, expected_jdbc_migrations);

        let initial_migration_checksum: i32 = sqlx::query_scalar(
            "SELECT checksum FROM flyway_schema_history WHERE version = '20200706141854'",
        )
        .fetch_one(&pool)
        .await
        .expect("Failed to read initial migration checksum");
        assert_eq!(initial_migration_checksum, -1_579_414_329);

        let ranks: Vec<i64> = sqlx::query_scalar(
            "SELECT installed_rank FROM flyway_schema_history ORDER BY installed_rank",
        )
        .fetch_all(&pool)
        .await
        .expect("Failed to read migration ranks");
        // Flyway in the original release expects gapless ranks starting at 1.
        let expected_ranks: Vec<i64> = (1..).take(ranks.len()).collect();
        assert_eq!(ranks, expected_ranks);
    }
}

#[tokio::test]
async fn test_transaction_rollback() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let db_path = temp_dir.path().join("database.sqlite");

    let db = Database::connect(&db_path)
        .await
        .expect("Failed to connect to SQLite");
    db.migrate().await.expect("Failed to run initial migration");
    let pool = db::open(&db_path).await;

    // Start a transaction, insert data, then roll back
    let mut tx = pool.begin().await.expect("Failed to begin tx");
    sqlx::query(
        "INSERT INTO USER (ID, EMAIL, PASSWORD) VALUES ('tx-user', 'tx@example.com', 'pwd')",
    )
    .execute(&mut *tx)
    .await
    .expect("Failed to insert in tx");

    tx.rollback().await.expect("Failed to rollback tx");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM USER WHERE ID = 'tx-user'")
        .fetch_one(&pool)
        .await
        .expect("Failed to count users");
    assert_eq!(count, 0, "Rolled back transaction must not persist data");
}

#[tokio::test]
async fn test_exclusive_directory_lock() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");

    // First lock acquisition succeeds
    let lock1 = DirectoryLock::acquire(temp_dir.path()).expect("First lock must succeed");

    // Second lock acquisition on the same directory fails
    let lock2_res = DirectoryLock::acquire(temp_dir.path());
    assert!(lock2_res.is_err(), "Concurrent lock must fail");
    assert!(
        DirectoryLock::try_acquire(temp_dir.path())
            .expect("Contended try_acquire must not report an I/O error")
            .is_none(),
        "Contended try_acquire must return Ok(None)"
    );

    drop(lock1);

    // After dropping lock1, lock acquisition succeeds again
    let _lock3 = DirectoryLock::try_acquire(temp_dir.path())
        .expect("Acquiring lock after release must not fail")
        .expect("Acquiring lock after release must succeed");
}

#[tokio::test]
async fn test_unknown_schema_version_rejected() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let db_path = temp_dir.path().join("database.sqlite");

    let db = Database::connect(&db_path)
        .await
        .expect("Failed to connect to SQLite");
    let pool = db::open(&db_path).await;

    // Create flyway_schema_history with an unknown / unsupported version
    create_flyway_schema_history(&pool).await;
    sqlx::query(
        "INSERT INTO flyway_schema_history (installed_rank, version, description, type, script, checksum, installed_by, execution_time, success) VALUES (1, '99999999999999', 'unknown migration', 'SQL', 'future.sql', 0, 'future', 1, 1)",
    )
    .execute(&pool)
    .await
    .expect("Failed to prepare unknown migration history");

    let migrate_res = db.migrate().await;
    match migrate_res {
        Err(DbError::UnknownVersion(v)) => {
            assert_eq!(v, "99999999999999");
        }
        other => panic!("Expected DbError::UnknownVersion, got: {other:?}"),
    }

    let history_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flyway_schema_history")
        .fetch_one(&pool)
        .await
        .expect("Failed to count migration history after rejection");
    let user_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'USER'",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to inspect schema after rejection");
    assert_eq!(
        history_count, 1,
        "Unknown schema history must remain unchanged"
    );
    assert_eq!(
        user_table_count, 0,
        "Unknown schema must not receive migrations"
    );
}

#[tokio::test]
async fn test_source_below_kotlin_migration_floor_is_rejected_without_writes() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let db_path = temp_dir.path().join("database.sqlite");
    let db = Database::connect(&db_path)
        .await
        .expect("Failed to connect to SQLite");
    let pool = db::open(&db_path).await;
    create_flyway_schema_history(&pool).await;
    sqlx::query(
        "INSERT INTO flyway_schema_history (installed_rank, version, description, type, script, installed_by, execution_time, success) VALUES (1, '20230801104436', 'fix incorrect language codes', 'JDBC', 'db.migration.sqlite.V20230801104436__fix_incorrect_language_codes', 'komga', 0, 1)",
    )
    .execute(&pool)
    .await
    .expect("Failed to prepare a below-floor schema history");

    let error = db
        .migrate()
        .await
        .expect_err("A source below the supported Kotlin migration floor must be rejected");
    assert!(
        matches!(&error, DbError::UnsupportedSchema(message) if message.contains("20240422132621") && message.contains("Kotlin Komga")),
        "Unexpected migration error: {error}"
    );

    let history_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flyway_schema_history")
        .fetch_one(&pool)
        .await
        .expect("Failed to count migration history after rejection");
    let application_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> 'flyway_schema_history'",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to inspect schema after rejection");
    assert_eq!(
        history_count, 1,
        "Rejected source history must stay unchanged"
    );
    assert_eq!(
        application_table_count, 0,
        "Rejected source must not be migrated"
    );
}

#[tokio::test]
async fn test_sql_checksum_mismatch_is_rejected_before_writes() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let db_path = temp_dir.path().join("database.sqlite");
    let db = Database::connect(&db_path)
        .await
        .expect("Failed to connect to SQLite");
    let pool = db::open(&db_path).await;
    create_flyway_schema_history(&pool).await;
    seed_jdbc_history(&pool).await;
    sqlx::query(
        "INSERT INTO flyway_schema_history (installed_rank, version, description, type, script, checksum, installed_by, execution_time, success) VALUES (6, '20240529120933', 'apikey', 'SQL', 'V20240529120933__apikey.sql', 0, 'komga', 1, 1)",
    )
    .execute(&pool)
    .await
    .expect("Failed to seed a mismatched SQL checksum");

    let error = db
        .migrate()
        .await
        .expect_err("A changed applied SQL migration must be rejected");
    assert!(
        matches!(&error, DbError::ChecksumMismatch { version, actual: Some(0), .. } if version == "20240529120933"),
        "Unexpected migration error: {error}"
    );

    let history_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flyway_schema_history")
        .fetch_one(&pool)
        .await
        .expect("Failed to count migration history after rejection");
    let application_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> 'flyway_schema_history'",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to inspect schema after rejection");
    assert_eq!(
        history_count, 6,
        "Rejected migration history must stay unchanged"
    );
    assert_eq!(
        application_table_count, 0,
        "Rejected source must not receive migrations"
    );
}

#[tokio::test]
async fn test_incomplete_history_is_rejected_without_replaying_old_migrations() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let db_path = temp_dir.path().join("database.sqlite");
    let db = Database::connect(&db_path)
        .await
        .expect("Failed to connect to SQLite");
    let pool = db::open(&db_path).await;
    create_flyway_schema_history(&pool).await;
    seed_jdbc_history(&pool).await;
    sqlx::query(
        "INSERT INTO flyway_schema_history (installed_rank, version, description, type, script, checksum, installed_by, execution_time, success) VALUES (6, '20240529120933', 'apikey', 'SQL', 'V20240529120933__apikey.sql', -936468172, 'komga', 1, 1)",
    )
    .execute(&pool)
    .await
    .expect("Failed to seed incomplete but checksummed migration history");

    let error = db
        .migrate()
        .await
        .expect_err("An incomplete applied migration history must be rejected");
    assert!(
        matches!(&error, DbError::UnsupportedSchema(message) if message.contains("missing successful version 20200706141854")),
        "Unexpected migration error: {error}"
    );

    let history_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM flyway_schema_history")
        .fetch_one(&pool)
        .await
        .expect("Failed to count migration history after rejection");
    let application_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> 'flyway_schema_history'",
    )
    .fetch_one(&pool)
    .await
    .expect("Failed to inspect schema after rejection");
    assert_eq!(
        history_count, 6,
        "Rejected migration history must stay unchanged"
    );
    assert_eq!(
        application_table_count, 0,
        "Rejected source must not receive migrations"
    );
}
