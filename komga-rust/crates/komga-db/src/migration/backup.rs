//! Pre-upgrade backup sets: every existing database copied with `VACUUM INTO`, which reads one
//! consistent snapshot through SQLite, WAL content included.

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqliteConnection};

use super::super::error::DbError;

/// Writes `<root>/pre-upgrade-<UTC time>/<name>` for each source and checks every copy opens as
/// an intact database. A set that cannot be completed is removed, so that no partial set looks
/// restorable.
pub(super) async fn create(
    sources: &mut [(&str, &mut SqliteConnection)],
    root: &Path,
) -> Result<PathBuf, DbError> {
    let set = root.join(format!(
        "pre-upgrade-{}",
        Timestamp::now().strftime("%Y%m%dT%H%M%S%.3fZ")
    ));
    std::fs::create_dir_all(root)
        .and_then(|()| std::fs::create_dir(&set))
        .map_err(|source| DbError::BackupDirectory {
            path: set.clone(),
            source,
        })?;
    for (name, conn) in sources.iter_mut() {
        let path = set.join(name);
        if let Err(error) = copy(conn, &path).await {
            let _ = std::fs::remove_dir_all(&set);
            return Err(error);
        }
    }
    tracing::info!(backup = %set.display(), "Created pre-migration database backup");
    Ok(set)
}

async fn copy(conn: &mut SqliteConnection, path: &Path) -> Result<(), DbError> {
    let backup_error = |source: sqlx::Error| DbError::Backup {
        path: path.to_path_buf(),
        source: Box::new(source),
    };
    let destination = path
        .to_str()
        .ok_or_else(|| DbError::BackupPath(path.to_path_buf()))?;
    sqlx::query("VACUUM INTO ?")
        .bind(destination)
        .execute(&mut *conn)
        .await
        .map_err(backup_error)?;

    let mut copy = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .connect()
        .await
        .map_err(backup_error)?;
    let check: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&mut copy)
        .await
        .map_err(backup_error)?;
    if check != "ok" {
        return Err(DbError::Backup {
            path: path.to_path_buf(),
            source: format!("the copy fails SQLite's quick check: {check}").into(),
        });
    }
    Ok(())
}
