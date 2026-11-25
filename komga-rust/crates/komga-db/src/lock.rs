//! Exclusive lock for a Komga data directory.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use super::error::DbError;

/// File name placed inside the locked Komga data directory.
const LOCK_FILE_NAME: &str = ".komga.lock";

/// Guard representing an exclusive lock on the database/data directory.
#[derive(Debug)]
pub struct DirectoryLock {
    _file: File,
    path: PathBuf,
}

impl DirectoryLock {
    /// Acquire an exclusive file lock in the given directory.
    ///
    /// # Errors
    /// Returns [`DbError::Lock`] if another instance holds the lock, or [`DbError::Io`] on
    /// filesystem failures.
    pub fn acquire(dir: &Path) -> Result<Self, DbError> {
        Self::try_acquire(dir)?.ok_or_else(|| {
            DbError::Lock(format!(
                "Directory is locked by another instance: {}",
                dir.join(LOCK_FILE_NAME).display()
            ))
        })
    }

    /// Try to acquire an exclusive file lock without waiting.
    ///
    /// Returns `Ok(None)` when another instance already holds the lock, distinguishing lock
    /// contention from underlying I/O errors so callers can retry safely.
    ///
    /// # Errors
    /// Returns [`DbError::Io`] if the directory or lock file cannot be created or locked.
    pub fn try_acquire(dir: &Path) -> Result<Option<Self>, DbError> {
        std::fs::create_dir_all(dir)?;
        let lock_path = dir.join(LOCK_FILE_NAME);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;

        match file.try_lock() {
            Ok(()) => Ok(Some(Self {
                _file: file,
                path: lock_path,
            })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => Err(DbError::Io(error)),
        }
    }

    /// Path to the lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}
