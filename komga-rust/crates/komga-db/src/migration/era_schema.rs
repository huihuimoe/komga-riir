//! Schema komga-rust owns in the main database: `migrations/era/`, recorded in
//! `ERA_SCHEMA_HISTORY`. It stays out of `flyway_schema_history`, so that Kotlin Komga still opens
//! the database and its later migrations never interleave with the `era` versions.

use sqlx::{AssertSqlSafe, Connection, SqliteConnection};
use tracing::info;

use super::super::error::DbError;
use super::embedded::{self, SqlMigration};

const HISTORY: &str = r#"
CREATE TABLE IF NOT EXISTS ERA_SCHEMA_HISTORY (
    VERSION TEXT PRIMARY KEY,
    DESCRIPTION TEXT NOT NULL,
    CHECKSUM INTEGER NOT NULL,
    INSTALLED_ON TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
)
"#;

/// The `era` migrations not applied yet. Read before Flyway runs, without creating the history
/// table, so that a fresh database is still seen as fresh. A version this build does not know
/// comes from a newer komga-rust.
pub(super) async fn pending(
    conn: &mut SqliteConnection,
) -> Result<Vec<&'static SqlMigration>, DbError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'ERA_SCHEMA_HISTORY')",
    )
    .fetch_one(&mut *conn)
    .await?;
    let applied: Vec<(String, i32)> = if exists {
        sqlx::query_as("SELECT VERSION, CHECKSUM FROM ERA_SCHEMA_HISTORY")
            .fetch_all(&mut *conn)
            .await?
    } else {
        Vec::new()
    };
    for (version, checksum) in &applied {
        let migration = embedded::ERA
            .iter()
            .find(|migration| &migration.version == version)
            .ok_or_else(|| DbError::UnknownVersion(format!("era {version}")))?;
        if migration.checksum != *checksum {
            return Err(DbError::ChecksumMismatch {
                version: format!("era {version}"),
                expected: migration.checksum,
                actual: Some(*checksum),
            });
        }
    }
    Ok(embedded::ERA
        .iter()
        .filter(|migration| {
            !applied
                .iter()
                .any(|(version, _)| *version == migration.version)
        })
        .collect())
}

pub(super) async fn apply(
    conn: &mut SqliteConnection,
    pending: &[&SqlMigration],
) -> Result<(), DbError> {
    sqlx::query(HISTORY).execute(&mut *conn).await?;
    for migration in pending {
        info!(
            version = migration.version,
            description = migration.description,
            "Applying komga-rust migration"
        );
        let mut tx = conn.begin().await?;
        sqlx::raw_sql(AssertSqlSafe(migration.sql.as_str()))
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO ERA_SCHEMA_HISTORY (VERSION, DESCRIPTION, CHECKSUM) VALUES (?, ?, ?)",
        )
        .bind(&migration.version)
        .bind(&migration.description)
        .bind(migration.checksum)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}
