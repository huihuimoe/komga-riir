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

/// Database handle encapsulating connection pool and migration state.
#[derive(Clone)]
pub struct Database(Arc<DatabaseInner>);

struct DatabaseInner {
    pool: Pool<Sqlite>,
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
            .busy_timeout(Duration::from_secs(30));
        if !read_only {
            options = options.synchronous(SqliteSynchronous::Normal);
        }

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options)
            .await?;

        Ok(Self(Arc::new(DatabaseInner {
            pool,
            db_path: db_path.to_path_buf(),
            sort_rules: Arc::new(SortRules::default()),
        })))
    }

    /// Initialize the SQLite connection pool without changing the database journal mode. Names
    /// sort by `sort_rules` on every connection.
    pub async fn connect_sorting_by(
        db_path: &Path,
        sort_rules: SortRules,
    ) -> Result<Self, DbError> {
        if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }

        let options = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            // Journal mode persists in the database, so migration validation must precede WAL setup.
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(30))
            .collation(collation::UNICODE_1, collation::compare_unicode_1);
        let options = sort_rules.register(options);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(30))
            .after_connect(|connection, _| {
                Box::pin(async move {
                    strip_accents::register(connection).await?;
                    regexp::register(connection).await
                })
            })
            .connect_with(options)
            .await?;

        Ok(Self(Arc::new(DatabaseInner {
            pool,
            db_path: db_path.to_path_buf(),
            sort_rules: Arc::new(sort_rules),
        })))
    }

    /// Begin a transaction for service-owned atomic operations.
    ///
    /// # Errors
    /// Returns an error if a SQLite connection cannot be acquired.
    pub async fn begin(&self) -> Result<DbTransaction, DbError> {
        let transaction = self.0.pool.begin().await?;
        Ok(self.transaction(transaction))
    }

    /// Begin a transaction that takes the write lock up front.
    ///
    /// Use it when a transaction reads before writing and concurrent callers must serialize:
    /// a deferred transaction fails with `SQLITE_BUSY` when upgrading to a write lock held by
    /// another, while an immediate one waits for the busy timeout.
    ///
    /// # Errors
    /// Returns an error if a SQLite connection or the write lock cannot be acquired.
    pub async fn begin_immediate(&self) -> Result<DbTransaction, DbError> {
        let transaction = self.0.pool.begin_with("BEGIN IMMEDIATE").await?;
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

    /// Access the SQLx pool inside the DB crate.
    #[must_use]
    pub(crate) fn pool(&self) -> &Pool<Sqlite> {
        &self.0.pool
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
            &[(&main.0.pool, Schema::Main), (&tasks.0.pool, Schema::Tasks)],
            backups,
        )
        .await
    }

    /// Migrates the main database alone, backing it up under `backups/` next to it first.
    ///
    /// # Errors
    /// As [`Self::migrate_all`].
    pub async fn migrate(&self) -> Result<(), DbError> {
        migration::migrate(&[(&self.0.pool, Schema::Main)], &self.sibling_backups()).await
    }

    /// Migrates the tasks database alone, backing it up under `backups/` next to it first.
    ///
    /// # Errors
    /// As [`Self::migrate_all`].
    pub async fn migrate_tasks(&self) -> Result<(), DbError> {
        migration::migrate(&[(&self.0.pool, Schema::Tasks)], &self.sibling_backups()).await
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
        sqlx::query("SELECT 1").execute(&self.0.pool).await?;
        Ok(())
    }

    /// Closes every connection, waiting for those in use; later queries fail.
    pub async fn close(&self) {
        self.0.pool.close().await;
    }
}
