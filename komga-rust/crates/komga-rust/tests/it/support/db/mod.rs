//! Direct access to the main database of a server under test, for what the API cannot show or
//! create yet and for injected failures. With [`super::tasks`] for the tasks database, it keeps
//! SQL out of the tests, except the `database` tests whose subject is the schema.

pub mod content;
pub mod seed;

use std::path::Path;
use std::time::Duration;

use tokio::time;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{AssertSqlSafe, Connection, SqliteConnection, SqlitePool};

pub async fn open(path: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(30));
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("Failed to connect to test SQLite database")
}

/// Read-only pool for shared template databases (never creates WAL/SHM files or takes write locks).
pub async fn open_read_only(path: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .busy_timeout(Duration::from_secs(30));
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("Failed to open read-only test SQLite database")
}

/// Disables rollback-journal disk flushes on a single-connection fixture pool while populating it.
/// `journal_mode = MEMORY` does not change the persistent database header mode.
pub async fn disable_sync(pool: &SqlitePool) {
    sqlx::raw_sql("PRAGMA journal_mode = MEMORY; PRAGMA synchronous = OFF;")
        .execute(pool)
        .await
        .expect("Configure fast fixture pragmas");
}

/// The main database of the data directory `directory`.
pub async fn main_pool(directory: &Path) -> SqlitePool {
    open(&directory.join("database.sqlite")).await
}

/// Polls the main database until `query` counts a row.
pub async fn wait_for_rows(main: &SqlitePool, query: &'static str) {
    time::timeout(Duration::from_secs(10), async {
        loop {
            let count: i64 = sqlx::query_scalar(query)
                .fetch_one(main)
                .await
                .expect("Count rows");
            if count > 0 {
                return;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("The rows appear within ten seconds");
}

/// Holds the main database write lock, which blocks workers inside their tasks, after the claim.
pub struct WriteLock(SqliteConnection);

impl WriteLock {
    pub async fn acquire(directory: &Path) -> Self {
        Self::on(&directory.join("database.sqlite")).await
    }

    pub async fn acquire_tasks(directory: &Path) -> Self {
        Self::on(&directory.join("tasks.sqlite")).await
    }

    async fn on(file: &Path) -> Self {
        let options = SqliteConnectOptions::new().filename(file);
        let mut connection = SqliteConnection::connect_with(&options)
            .await
            .expect("Open database");
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut connection)
            .await
            .expect("Lock database");
        Self(connection)
    }

    pub async fn release(mut self) {
        sqlx::query("ROLLBACK")
            .execute(&mut self.0)
            .await
            .expect("Release database");
        self.0.close().await.expect("Close database");
    }
}

/// Aborts the statements `condition` matches, such as `BEFORE DELETE ON LIBRARY`, until
/// [`remove_failure`].
pub async fn inject_failure(pool: &SqlitePool, condition: &str) {
    sqlx::query(AssertSqlSafe(format!(
        "CREATE TRIGGER injected_failure {condition} BEGIN SELECT RAISE(ABORT, 'injected'); END"
    )))
    .execute(pool)
    .await
    .expect("Inject failure");
}

pub async fn remove_failure(pool: &SqlitePool) {
    sqlx::query("DROP TRIGGER injected_failure")
        .execute(pool)
        .await
        .expect("Remove injected failure");
}

pub async fn server_setting(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT VALUE FROM SERVER_SETTINGS WHERE KEY = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .expect("Read server setting")
        .flatten()
}

pub async fn user_emails(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar("SELECT EMAIL FROM USER")
        .fetch_all(pool)
        .await
        .expect("Failed to read users")
}

/// The stored root of the only library.
pub async fn library_root(pool: &SqlitePool) -> String {
    sqlx::query_scalar("SELECT ROOT FROM LIBRARY")
        .fetch_one(pool)
        .await
        .expect("Stored root")
}
