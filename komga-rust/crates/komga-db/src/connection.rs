//! SQLite pool setup and database entry points.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use komga_types::DomainEvent;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Pool, Sqlite, SqliteConnection, Transaction};

use super::error::DbError;
use super::migration::{self, Schema};
use super::sqlite::collation::{self, SortRules};
use super::sqlite::{regexp, strip_accents};

/// Known baseline initial migration version.
pub const INITIAL_SCHEMA_VERSION: &str = "20200706141854";

/// Maximum concurrent read-only connections per database file.
const READ_CONNECTIONS: u32 = 5;
/// Single writer connection: SQLite allows one writer at a time, so queuing concurrent writes in
/// the pool prevents waiting writers from exhausting reader connections.
const WRITE_CONNECTIONS: u32 = 1;
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);

/// Database handle encapsulating separated write and read-only SQLite connection pools.
#[derive(Clone)]
pub struct Database(Arc<DatabaseInner>);

struct DatabaseInner {
    write_pool: Pool<Sqlite>,
    read_pool: Pool<Sqlite>,
    db_path: PathBuf,
    sort_rules: Arc<SortRules>,
}

/// A database transaction whose SQLx connection stays private to the DB crate.
pub struct DbTransaction {
    transaction: Transaction<'static, Sqlite>,
    sort_rules: Arc<SortRules>,
    events: Vec<DomainEvent>,
}

impl DbTransaction {
    /// Commit all writes performed through this transaction, which must have recorded no events.
    ///
    /// # Errors
    /// Returns the underlying database execution failure without exposing SQLx.
    pub async fn commit(self) -> Result<(), DbError> {
        debug_assert!(
            self.events.is_empty(),
            "Recorded events must be published through the commit returning them"
        );
        self.transaction.commit().await?;
        Ok(())
    }

    /// Commits like [`Self::commit`] and returns the events recorded for the caller to publish.
    ///
    /// # Errors
    /// Returns the underlying database execution failure without exposing SQLx.
    pub async fn commit_with_events(self) -> Result<Vec<DomainEvent>, DbError> {
        self.transaction.commit().await?;
        Ok(self.events)
    }

    /// Holds a change made in this transaction until it commits; a rollback drops it. The DB
    /// never records events itself.
    pub fn record(&mut self, event: DomainEvent) {
        self.events.push(event);
    }

    /// Roll back all writes performed through this transaction.
    ///
    /// # Errors
    /// Returns the underlying database execution failure without exposing SQLx.
    pub async fn rollback(self) -> Result<(), DbError> {
        self.transaction.rollback().await?;
        Ok(())
    }

    pub(crate) fn connection(&mut self) -> &mut SqliteConnection {
        &mut self.transaction
    }

    pub(crate) fn sort_rules(&self) -> &SortRules {
        &self.sort_rules
    }
}

impl Database {
    /// [`connect_sorting_by`](Self::connect_sorting_by) the root locale.
    pub async fn connect(db_path: &Path) -> Result<Self, DbError> {
        Self::connect_sorting_by(db_path, SortRules::default()).await
    }

    /// Open an existing SQLite database for reading and writing without creating it.
    pub async fn connect_existing(db_path: &Path) -> Result<Self, DbError> {
        Self::connect_existing_with(db_path, false).await
    }

    /// Open an existing SQLite database in read-only mode without creating it.
    pub async fn connect_existing_read_only(db_path: &Path) -> Result<Self, DbError> {
        Self::connect_existing_with(db_path, true).await
    }

    async fn connect_existing_with(db_path: &Path, read_only: bool) -> Result<Self, DbError> {
        let mut options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(false)
            .read_only(read_only)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        if !read_only {
            options = options.synchronous(SqliteSynchronous::Normal);
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .connect_with(options)
            .await?;

        Ok(Self(Arc::new(DatabaseInner {
            write_pool: pool.clone(),
            read_pool: pool,
            db_path: db_path.to_path_buf(),
            sort_rules: Arc::new(SortRules::default()),
        })))
    }

    /// Initialize separated write and read-only SQLite connection pools without changing the
    /// database journal mode. Names sort by `sort_rules` on every connection.
    pub async fn connect_sorting_by(
        db_path: &Path,
        sort_rules: SortRules,
    ) -> Result<Self, DbError> {
        if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }

        let write_options = base_options(db_path, &sort_rules)
            .create_if_missing(true)
            // Journal mode persists in the database, so migration validation must precede WAL setup.
            .synchronous(SqliteSynchronous::Normal);
        let write_pool = pool_options(WRITE_CONNECTIONS)
            .connect_with(write_options)
            .await?;

        // Opened lazily so migrations can back up the file and switch to WAL while only the
        // writer connection is open. `read_only(true)` still permits per-connection `temp` tables
        // (used by full-text search candidates) while rejecting writes to the main database.
        let read_options = base_options(db_path, &sort_rules)
            .create_if_missing(false)
            .read_only(true);
        let read_pool = pool_options(READ_CONNECTIONS).connect_lazy_with(read_options);

        Ok(Self(Arc::new(DatabaseInner {
            write_pool,
            read_pool,
            db_path: db_path.to_path_buf(),
            sort_rules: Arc::new(sort_rules),
        })))
    }

    /// Begin a read-only transaction on the reader pool.
    ///
    /// Statements in this transaction can read the database and create temporary tables, but
    /// writes to the database fail with `SQLITE_READONLY`. Use [`Self::begin_immediate`] for
    /// transactions that write.
    ///
    /// # Errors
    /// Returns an error if a SQLite reader connection cannot be acquired.
    pub async fn begin(&self) -> Result<DbTransaction, DbError> {
        let transaction = self.0.read_pool.begin().await?;
        Ok(self.transaction(transaction))
    }

    /// Begin a write transaction on the dedicated single-connection writer pool, taking the write
    /// lock up front (`BEGIN IMMEDIATE`).
    ///
    /// Concurrent writers wait on the writer pool without consuming reader connections, so write
    /// contention cannot starve read requests.
    ///
    /// # Errors
    /// Returns an error if the writer connection or the write lock cannot be acquired.
    pub async fn begin_immediate(&self) -> Result<DbTransaction, DbError> {
        let transaction = self.0.write_pool.begin_with("BEGIN IMMEDIATE").await?;
        Ok(self.transaction(transaction))
    }

    fn transaction(&self, transaction: Transaction<'static, Sqlite>) -> DbTransaction {
        DbTransaction {
            transaction,
            sort_rules: Arc::clone(&self.0.sort_rules),
            events: Vec::new(),
        }
    }

    pub(crate) fn sort_rules(&self) -> &SortRules {
        &self.0.sort_rules
    }

    /// Access the read-only SQLx pool inside the DB crate.
    #[must_use]
    pub(crate) fn read_pool(&self) -> &Pool<Sqlite> {
        &self.0.read_pool
    }

    /// Access the single-connection write SQLx pool inside the DB crate.
    #[must_use]
    pub(crate) fn write_pool(&self) -> &Pool<Sqlite> {
        &self.0.write_pool
    }

    /// Migrates the main and tasks databases as a pair: both histories are checked before either
    /// is written, and an upgrade of existing data first backs both up into one set under
    /// `backups`.
    ///
    /// # Errors
    /// Rejects unknown sources and versions, unsupported schemas and changed SQL migrations before
    /// writing; fails without migrating when the backup set cannot be created.
    pub async fn migrate_all(main: &Self, tasks: &Self, backups: &Path) -> Result<(), DbError> {
        migration::migrate(
            &[
                (&main.0.write_pool, Schema::Main),
                (&tasks.0.write_pool, Schema::Tasks),
            ],
            backups,
        )
        .await
    }

    /// Migrates the main database alone, backing it up under `backups/` next to it first.
    ///
    /// # Errors
    /// As [`Self::migrate_all`].
    pub async fn migrate(&self) -> Result<(), DbError> {
        migration::migrate(
            &[(&self.0.write_pool, Schema::Main)],
            &self.sibling_backups(),
        )
        .await
    }

    /// Migrates the tasks database alone, backing it up under `backups/` next to it first.
    ///
    /// # Errors
    /// As [`Self::migrate_all`].
    pub async fn migrate_tasks(&self) -> Result<(), DbError> {
        migration::migrate(
            &[(&self.0.write_pool, Schema::Tasks)],
            &self.sibling_backups(),
        )
        .await
    }

    fn sibling_backups(&self) -> PathBuf {
        self.0
            .db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("backups")
    }

    /// Whether a connection can run a query; it proves reads only.
    ///
    /// # Errors
    /// Returns the failure to acquire a connection or to run the query.
    pub async fn ping(&self) -> Result<(), DbError> {
        sqlx::query("SELECT 1").execute(&self.0.read_pool).await?;
        Ok(())
    }

    /// Closes every connection, waiting for those in use; later queries fail.
    pub async fn close(&self) {
        self.0.read_pool.close().await;
        self.0.write_pool.close().await;
    }
}

fn base_options(db_path: &Path, sort_rules: &SortRules) -> SqliteConnectOptions {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .foreign_keys(true)
        .busy_timeout(BUSY_TIMEOUT)
        .collation(collation::UNICODE_1, collation::compare_unicode_1);
    sort_rules.register(options)
}

fn pool_options(max_connections: u32) -> SqlitePoolOptions {
    SqlitePoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .after_connect(|connection, _| {
            Box::pin(async move {
                strip_accents::register(connection).await?;
                regexp::register(connection).await
            })
        })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::oneshot;
    use tokio::time;

    use super::{Database, READ_CONNECTIONS};
    use crate::{SearchCandidates, ServerSettingsDao};

    #[tokio::test]
    async fn queued_writers_do_not_starve_concurrent_readers() {
        let directory = komga_test_support::tempdir().expect("Database directory");
        let db = Database::connect(&directory.path().join("database.sqlite"))
            .await
            .expect("Open database");
        db.migrate().await.expect("Migrate database");

        let (writer_entered_tx, writer_entered_rx) = oneshot::channel();
        let (release_writer_tx, release_writer_rx) = oneshot::channel();
        let holder_db = db.clone();
        let holder = tokio::spawn(async move {
            let mut tx = holder_db.begin_immediate().await.expect("Acquire writer");
            ServerSettingsDao::save(&mut tx, "HELD", "1")
                .await
                .expect("Write inside transaction");
            let _ = writer_entered_tx.send(());
            let _ = release_writer_rx.await;
            tx.commit().await.expect("Commit held write");
        });
        writer_entered_rx.await.expect("Holder acquired write lock");

        // Queue more writers than the reader pool has connections; none may occupy a reader slot.
        let mut queued = Vec::new();
        for index in 0..=(READ_CONNECTIONS + 1) {
            let writer_db = db.clone();
            queued.push(tokio::spawn(async move {
                let mut tx = writer_db
                    .begin_immediate()
                    .await
                    .expect("Queued writer acquires write lock once released");
                ServerSettingsDao::save(&mut tx, &format!("K{index}"), "v")
                    .await
                    .expect("Queued write succeeds");
                tx.commit().await.expect("Commit queued write");
            }));
        }
        // Let queued writers register on the writer pool semaphore.
        tokio::task::yield_now().await;

        // Every reader slot must still be immediately usable while all writers wait.
        let mut readers = Vec::new();
        for _ in 0..READ_CONNECTIONS {
            let mut tx = time::timeout(Duration::from_secs(2), db.begin())
                .await
                .expect("Reader must not wait on queued writers")
                .expect("Begin read transaction");
            let one: i64 = sqlx::query_scalar("SELECT 1")
                .fetch_one(tx.connection())
                .await
                .expect("Read query succeeds");
            assert_eq!(one, 1);
            readers.push(tx);
        }
        drop(readers);

        time::timeout(Duration::from_secs(2), db.ping())
            .await
            .expect("Ping must not wait on queued writers")
            .expect("Ping succeeds");

        let _ = release_writer_tx.send(());
        holder.await.expect("Holder task");
        for task in queued {
            task.await.expect("Queued writer task");
        }
        assert_eq!(
            ServerSettingsDao::get(&db, "HELD")
                .await
                .expect("Read committed value")
                .as_deref(),
            Some("1")
        );
    }

    #[tokio::test]
    async fn read_transactions_reject_main_writes_and_allow_temp_tables() {
        let directory = komga_test_support::tempdir().expect("Database directory");
        let db = Database::connect(&directory.path().join("database.sqlite"))
            .await
            .expect("Open database");
        db.migrate().await.expect("Migrate database");

        let mut tx = db.begin().await.expect("Begin read transaction");
        let mut candidates = SearchCandidates::create(&mut tx)
            .await
            .expect("Temp table creation works on read-only connection");
        candidates
            .insert(&mut tx, &["id-1".to_owned(), "id-2".to_owned()])
            .await
            .expect("Temp table insert works on read-only connection");
        candidates
            .analyze(&mut tx)
            .await
            .expect("Temp table analyze works on read-only connection");

        let write_err = ServerSettingsDao::save(&mut tx, "RO_WRITE", "nope").await;
        assert!(
            write_err.is_err(),
            "Main database writes must fail on read-only transactions"
        );
    }
}
