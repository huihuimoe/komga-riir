use std::time::Instant;

use sqlx::{AssertSqlSafe, Connection, SqliteConnection};
use tracing::info;

use super::super::error::DbError;
use super::validate::next_migration;
use super::{Migration, Plan, history};

pub(super) async fn run_flyway_migrations(plan: &mut Plan) -> Result<(), DbError> {
    let conn = plan.conn.as_mut();
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode = WAL")
        .fetch_one(&mut *conn)
        .await?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(DbError::JournalMode(journal_mode));
    }

    history::create_table(conn).await?;

    let mut current_rank: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(installed_rank), 0) FROM flyway_schema_history")
            .fetch_one(&mut *conn)
            .await?;

    let sql_migrations = plan.schema.sql_migrations();
    let jdbc_migrations = plan.schema.jdbc_migrations();
    let mut sql_index = 0;
    let mut jdbc_index = 0;
    while let Some(migration) = next_migration(
        sql_migrations,
        &mut sql_index,
        jdbc_migrations,
        &mut jdbc_index,
    ) {
        if plan
            .entries
            .iter()
            .any(|entry| entry.version.as_deref() == Some(migration.version()) && entry.success)
        {
            continue;
        }

        current_rank += 1;
        apply_migration(conn, migration, current_rank).await?;
    }

    Ok(())
}

async fn apply_migration(
    conn: &mut SqliteConnection,
    migration: Migration<'_>,
    installed_rank: i64,
) -> Result<(), DbError> {
    info!(
        version = migration.version(),
        migration_type = migration.migration_type(),
        description = migration.description(),
        "Applying Flyway migration"
    );
    // SQLite ignores `PRAGMA foreign_keys` inside a transaction, so Flyway (`mixed`) runs such a
    // migration outside one. Turning enforcement off around the transaction has the same effect
    // and keeps the migration atomic.
    let sets_foreign_keys = migration
        .sql()
        .is_some_and(|sql| sql.to_ascii_uppercase().contains("PRAGMA FOREIGN_KEYS"));
    if sets_foreign_keys {
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *conn)
            .await?;
    }
    let applied = apply_in_transaction(conn, migration, installed_rank).await;
    if sets_foreign_keys {
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *conn)
            .await?;
    }
    applied
}

async fn apply_in_transaction(
    conn: &mut SqliteConnection,
    migration: Migration<'_>,
    installed_rank: i64,
) -> Result<(), DbError> {
    let start = Instant::now();
    let mut tx = conn.begin().await?;

    if let Some(sql) = migration.sql() {
        sqlx::raw_sql(AssertSqlSafe(sql)).execute(&mut *tx).await?;
    }

    let elapsed_ms = i32::try_from(start.elapsed().as_millis()).unwrap_or(i32::MAX);
    record(&mut tx, migration, installed_rank, "komga-rust", elapsed_ms).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn record(
    conn: &mut SqliteConnection,
    migration: Migration<'_>,
    installed_rank: i64,
    installed_by: &str,
    execution_ms: i32,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO flyway_schema_history (
            installed_rank, version, description, type, script, checksum, installed_by, execution_time, success
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1)
        "#,
    )
    .bind(installed_rank)
    .bind(migration.version())
    .bind(migration.description())
    .bind(migration.migration_type())
    .bind(migration.script())
    .bind(migration.checksum())
    .bind(installed_by)
    .bind(execution_ms)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
