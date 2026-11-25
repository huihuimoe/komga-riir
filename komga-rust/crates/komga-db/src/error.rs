//! Errors originating from SQLite storage.

use std::error::Error;
use std::path::PathBuf;

use thiserror::Error;

/// Errors originating from the database subsystem.
#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SQL execution error: {0}")]
    Execution(#[source] Box<dyn Error + Send + Sync>),

    #[error("Database lock acquisition failed: {0}")]
    Lock(String),

    #[error("Unsupported or unknown database schema version: {0}")]
    UnknownVersion(String),

    #[error("Unsupported database schema: {0}")]
    UnsupportedSchema(String),

    #[error("Migration {0} has a failed history entry; refusing to continue")]
    FailedMigration(String),

    #[error(
        "Flyway checksum mismatch for migration {version}: expected {expected}, found {actual:?}"
    )]
    ChecksumMismatch {
        version: String,
        expected: i32,
        actual: Option<i32>,
    },

    #[error("SQLite did not enable WAL mode; journal mode is {0}")]
    JournalMode(String),

    #[error("Failed to create backup directory {path}: {source}")]
    BackupDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to create pre-migration backup at {path}: {source}")]
    Backup {
        path: PathBuf,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },

    #[error("Backup path cannot be represented as UTF-8: {0}")]
    BackupPath(PathBuf),

    #[error(
        "Migration failed after backing up the databases to {backup}; restore them from there to return to the previous version"
    )]
    MigrationAfterBackup {
        backup: PathBuf,
        #[source]
        source: Box<DbError>,
    },
}

impl From<sqlx::Error> for DbError {
    fn from(error: sqlx::Error) -> Self {
        Self::Execution(Box::new(error))
    }
}
