use std::fs;
use std::path::{Path, PathBuf};

use komga_db::{Database, DbError};
use komga_test_support::TestDir;

use crate::support::db;

const TASKS_INITIAL_MIGRATION: &str =
    include_str!("../../../../komga-db/migrations/tasks/V20231013114850__tasks.sql");
const TASKS_PENDING_VERSION: &str = "20261002175358";
const CREATE_FLYWAY_SCHEMA_HISTORY: &str = r#"
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

/// A data directory as an older Komga leaves it: a current main database holding a setting that
/// is still in its WAL, and a tasks database one migration behind.
struct OldDataDirectory {
    main: Database,
    tasks: Database,
    dir: TestDir,
}

impl OldDataDirectory {
    async fn create() -> Self {
        let dir = komga_test_support::tempdir().expect("Data directory");
        let main = Database::connect(&dir.path().join("database.sqlite"))
            .await
            .expect("Connect to the main database");
        main.migrate().await.expect("Migrate the main database");
        let main_pool = db::main_pool(dir.path()).await;
        sqlx::query("INSERT INTO SERVER_SETTINGS (KEY, VALUE) VALUES ('backup-probe', 'kept')")
            .execute(&main_pool)
            .await
            .expect("Write a setting");
        main_pool.close().await;

        let tasks = Database::connect(&dir.path().join("tasks.sqlite"))
            .await
            .expect("Connect to the tasks database");
        let tasks_pool = db::open(&dir.path().join("tasks.sqlite")).await;
        sqlx::raw_sql(TASKS_INITIAL_MIGRATION)
            .execute(&tasks_pool)
            .await
            .expect("Apply the first tasks migration");
        sqlx::query(CREATE_FLYWAY_SCHEMA_HISTORY)
            .execute(&tasks_pool)
            .await
            .expect("Create the Flyway history");
        sqlx::query(
            "INSERT INTO flyway_schema_history (installed_rank, version, description, type, script, checksum, installed_by, execution_time, success) VALUES (1, '20231013114850', 'tasks', 'SQL', 'V20231013114850__tasks.sql', 1745982750, 'komga', 1, 1)",
        )
        .execute(&tasks_pool)
        .await
        .expect("Record the first tasks migration");
        tasks_pool.close().await;
        Self { dir, main, tasks }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    async fn migrate(&self) -> Result<(), DbError> {
        Database::migrate_all(&self.main, &self.tasks, &self.path("backups")).await
    }
}

fn entries(directory: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<_> = fs::read_dir(directory)
        .expect("Read directory")
        .map(|entry| entry.expect("Directory entry").path())
        .collect();
    entries.sort();
    entries
}

async fn tasks_migrated(path: &Path) -> bool {
    let pool = db::open(path).await;
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM flyway_schema_history WHERE version = ?")
            .bind(TASKS_PENDING_VERSION)
            .fetch_one(&pool)
            .await
            .expect("Read the Flyway history");
    pool.close().await;
    count == 1
}

async fn setting(path: &Path) -> Option<String> {
    let pool = db::open(path).await;
    let value = sqlx::query_scalar("SELECT VALUE FROM SERVER_SETTINGS WHERE KEY = 'backup-probe'")
        .fetch_optional(&pool)
        .await
        .expect("Read the setting");
    pool.close().await;
    value
}

#[tokio::test]
async fn an_upgrade_backs_up_both_databases_into_one_restorable_set() {
    let data = OldDataDirectory::create().await;
    data.migrate().await.expect("Upgrade after the backup");
    assert!(tasks_migrated(&data.path("tasks.sqlite")).await);

    let sets = entries(&data.path("backups"));
    assert_eq!(sets.len(), 1, "{sets:?}");
    let set_name = sets[0].file_name().and_then(|name| name.to_str());
    assert!(
        set_name.is_some_and(|name| name.starts_with("pre-upgrade-")),
        "{set_name:?}"
    );
    assert_eq!(
        entries(&sets[0]),
        [
            sets[0].join("database.sqlite"),
            sets[0].join("tasks.sqlite")
        ]
    );

    // Restore as the documented procedure does: stop, copy each file back, drop WAL leftovers.
    data.main.close().await;
    data.tasks.close().await;
    for name in ["database.sqlite", "tasks.sqlite"] {
        fs::copy(sets[0].join(name), data.path(name)).expect("Restore the database");
        for suffix in ["-wal", "-shm"] {
            let _ = fs::remove_file(data.path(&format!("{name}{suffix}")));
        }
    }
    assert_eq!(
        setting(&data.path("database.sqlite")).await.as_deref(),
        Some("kept")
    );
    assert!(!tasks_migrated(&data.path("tasks.sqlite")).await);

    let main = Database::connect(&data.path("database.sqlite")).await;
    let tasks = Database::connect(&data.path("tasks.sqlite")).await;
    let restored = (main.expect("Reopen main"), tasks.expect("Reopen tasks"));
    Database::migrate_all(&restored.0, &restored.1, &data.path("backups"))
        .await
        .expect("The restored pair upgrades again");
}

#[tokio::test]
async fn an_unknown_source_stops_the_upgrade_before_anything_is_written() {
    let data = OldDataDirectory::create().await;
    let main_pool = db::main_pool(data.dir.path()).await;
    sqlx::query(
        "INSERT INTO flyway_schema_history (installed_rank, version, description, type, script, installed_by, execution_time, success) \
         VALUES (1000, '99990101000000', 'newer', 'SQL', 'V99990101000000__newer.sql', 'komga', 0, 1)",
    )
    .execute(&main_pool)
    .await
    .expect("Record a migration from a newer Komga");
    main_pool.close().await;

    let error = data
        .migrate()
        .await
        .expect_err("Databases from a newer Komga are refused");
    assert!(
        matches!(&error, DbError::UnknownVersion(version) if version == "99990101000000"),
        "{error}"
    );
    assert!(!data.path("backups").exists());
    assert!(!tasks_migrated(&data.path("tasks.sqlite")).await);
}

#[tokio::test]
async fn a_failed_backup_stops_the_upgrade() {
    let data = OldDataDirectory::create().await;
    fs::write(data.path("backups"), b"not a directory").expect("Block the backup directory");

    let error = data
        .migrate()
        .await
        .expect_err("No upgrade without a backup");
    assert!(matches!(&error, DbError::BackupDirectory { .. }), "{error}");
    assert!(!tasks_migrated(&data.path("tasks.sqlite")).await);
}

#[tokio::test]
async fn an_unchanged_or_fresh_pair_takes_no_backup() {
    let dir = komga_test_support::tempdir().expect("Data directory");
    let main = Database::connect(&dir.path().join("database.sqlite")).await;
    let tasks = Database::connect(&dir.path().join("tasks.sqlite")).await;
    let (main, tasks) = (main.expect("Main"), tasks.expect("Tasks"));
    let backups = dir.path().join("backups");
    for _ in 0..2 {
        Database::migrate_all(&main, &tasks, &backups)
            .await
            .expect("Migrate");
    }
    assert!(!backups.exists());
}
