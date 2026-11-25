//! Databases last migrated by the legacy komga-rust (the earlier port this workspace replaced
//! and took the name of), kept so its users can upgrade.
//!
//! Legacy komga-rust applies Kotlin's SQL migrations through SQLx and records them only in
//! `_sqlx_migrations`, never runs the JDBC migrations, and stores plain file paths where Kotlin
//! stores file URLs. Adoption rebuilds the Flyway history from its record and converts the paths,
//! after the pre-upgrade backup. Its separate `riir.sqlite` holds no Komga data and stays as is.

use std::path::Path;

use komga_types::FileUrl;
use sqlx::{AssertSqlSafe, Connection, SqliteConnection};

use super::super::error::DbError;
use super::history::{self, Entry, Snapshot};
use super::{Migration, Schema, next_migration, record};

const INSTALLED_BY: &str = "komga-rust";

/// Each file column with the query selecting its unconverted paths and whether each names a
/// directory, which Java's file URLs mark with a trailing `/`.
const FILE_COLUMNS: &[(&str, &str, &str)] = &[
    (
        "LIBRARY",
        "ROOT",
        "SELECT DISTINCT ROOT, 1 FROM LIBRARY WHERE ROOT NOT LIKE 'file:%'",
    ),
    (
        "SERIES",
        "URL",
        "SELECT DISTINCT URL, NOT ONESHOT FROM SERIES WHERE URL NOT LIKE 'file:%'",
    ),
    (
        "BOOK",
        "URL",
        "SELECT DISTINCT URL, 0 FROM BOOK WHERE URL NOT LIKE 'file:%'",
    ),
    (
        "SIDECAR",
        "URL",
        "SELECT DISTINCT URL, 0 FROM SIDECAR WHERE URL NOT LIKE 'file:%'",
    ),
    (
        "SIDECAR",
        "PARENT_URL",
        "SELECT DISTINCT PARENT_URL, PARENT_URL NOT IN (SELECT URL FROM BOOK) FROM SIDECAR WHERE PARENT_URL NOT LIKE 'file:%'",
    ),
    (
        "THUMBNAIL_BOOK",
        "URL",
        "SELECT DISTINCT URL, 0 FROM THUMBNAIL_BOOK WHERE URL NOT LIKE 'file:%'",
    ),
    (
        "THUMBNAIL_SERIES",
        "URL",
        "SELECT DISTINCT URL, 0 FROM THUMBNAIL_SERIES WHERE URL NOT LIKE 'file:%'",
    ),
];

pub(super) struct Adoption {
    missing: Vec<Migration<'static>>,
    file_urls: Vec<FileUrlRewrite>,
}

struct FileUrlRewrite {
    table: &'static str,
    column: &'static str,
    path: String,
    url: FileUrl,
}

#[derive(sqlx::FromRow)]
struct SqlxEntry {
    version: i64,
    success: bool,
}

/// The Flyway history the database has once adopted, or `None` when komga-rust never migrated
/// it. Read only.
pub(super) async fn read(
    conn: &mut SqliteConnection,
    schema: Schema,
) -> Result<Option<(Snapshot, Adoption)>, DbError> {
    let migrated: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(&mut *conn)
    .await?;
    if !migrated {
        return Ok(None);
    }
    let applied: Vec<SqlxEntry> =
        sqlx::query_as("SELECT version, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&mut *conn)
            .await?;
    let mut versions = Vec::with_capacity(applied.len());
    for entry in applied {
        let version = entry.version.to_string();
        if !entry.success {
            return Err(DbError::FailedMigration(version));
        }
        if !schema
            .sql_migrations()
            .iter()
            .any(|migration| migration.version == version)
        {
            return Err(DbError::UnknownVersion(format!("komga-rust {version}")));
        }
        versions.push(version);
    }
    let Some(latest) = versions.last().cloned() else {
        return Err(DbError::UnsupportedSchema(
            "komga-rust migration history is empty".to_owned(),
        ));
    };

    // A Flyway history means komga-rust adopted a Kotlin database, whose JDBC migrations must
    // already be there; without one it created the database, so they had no data to change.
    let flyway = history::entries(conn).await?;
    let jdbc_ran_on_empty_data = flyway.is_none();
    let mut entries = flyway.unwrap_or_default();
    let mut missing = Vec::new();
    let (mut sql_index, mut jdbc_index) = (0, 0);
    while let Some(migration) = next_migration(
        schema.sql_migrations(),
        &mut sql_index,
        schema.jdbc_migrations(),
        &mut jdbc_index,
    ) {
        if migration.version() > latest.as_str() {
            break;
        }
        let recorded = entries
            .iter()
            .any(|entry| entry.version.as_deref() == Some(migration.version()));
        let applied = match migration {
            Migration::Sql(sql) => versions.contains(&sql.version),
            Migration::Jdbc(_) => jdbc_ran_on_empty_data,
        };
        if !recorded && applied {
            missing.push(migration);
        }
    }
    entries.extend(missing.iter().map(|&migration| Entry {
        version: Some(migration.version().to_owned()),
        migration_type: migration.migration_type().to_owned(),
        checksum: migration.checksum(),
        success: true,
    }));
    Ok(Some((
        Snapshot {
            entries,
            is_fresh: false,
        },
        Adoption {
            missing,
            file_urls: Vec::new(),
        },
    )))
}

impl Adoption {
    /// Collects the stored paths to convert; needs a schema whose history has been validated.
    pub(super) async fn read_file_paths(
        &mut self,
        conn: &mut SqliteConnection,
    ) -> Result<(), DbError> {
        for &(table, column, query) in FILE_COLUMNS {
            let paths: Vec<(String, bool)> = sqlx::query_as(AssertSqlSafe(query))
                .fetch_all(&mut *conn)
                .await?;
            for (path, directory) in paths {
                let file = Path::new(&path);
                if !file.is_absolute() {
                    return Err(DbError::UnsupportedSchema(format!(
                        "komga-rust stored the relative path `{path}` in {table}.{column}"
                    )));
                }
                let url = if directory {
                    FileUrl::directory(file)
                } else {
                    FileUrl::file(file)
                };
                self.file_urls.push(FileUrlRewrite {
                    table,
                    column,
                    path,
                    url,
                });
            }
        }
        Ok(())
    }

    /// Records the history in Flyway's table, converts the paths, and drops komga-rust's history,
    /// in one transaction.
    pub(super) async fn apply(self, conn: &mut SqliteConnection) -> Result<(), DbError> {
        let mut tx = conn.begin().await?;
        history::create_table(&mut tx).await?;
        let mut rank: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(installed_rank), 0) FROM flyway_schema_history",
        )
        .fetch_one(&mut *tx)
        .await?;
        for migration in self.missing {
            rank += 1;
            record(&mut tx, migration, rank, INSTALLED_BY, 0).await?;
        }
        for rewrite in &self.file_urls {
            let update = format!(
                "UPDATE {table} SET {column} = ? WHERE {column} = ?",
                table = rewrite.table,
                column = rewrite.column
            );
            sqlx::query(AssertSqlSafe(update))
                .bind(rewrite.url.as_str())
                .bind(&rewrite.path)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DROP TABLE _sqlx_migrations")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::info!(
            file_urls = self.file_urls.len(),
            "Adopted a database migrated by komga-rust"
        );
        Ok(())
    }
}
