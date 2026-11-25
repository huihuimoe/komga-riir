//! DAO for persisted server settings.

use std::collections::HashMap;

use crate::iden::ServerSettingsTable as S;
use sea_query::{Expr, ExprTrait, OnConflict, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::connection::{Database, DbTransaction};
use crate::error::DbError;

/// Read and write values in `SERVER_SETTINGS`.
pub struct ServerSettingsDao;

impl ServerSettingsDao {
    /// Read a setting, returning `None` when its row or value is absent.
    pub async fn get(database: &Database, key: &str) -> Result<Option<String>, DbError> {
        let (sql, values) = Query::select()
            .column(S::Value)
            .from(S::Table)
            .and_where(Expr::col(S::Key).eq(key))
            .build_sqlx(SqliteQueryBuilder);
        let value: Option<Option<String>> =
            sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_optional(database.pool())
                .await?;
        Ok(value.flatten())
    }

    /// Insert a default only when the row or its value is absent, then return the stored value.
    pub async fn get_or_insert(
        database: &Database,
        key: &str,
        default_value: &str,
    ) -> Result<String, DbError> {
        let (sql, values) = Query::insert()
            .into_table(S::Table)
            .columns([S::Key, S::Value])
            .values_panic([key.into(), default_value.into()])
            .on_conflict(
                OnConflict::column(S::Key)
                    .update_column(S::Value)
                    .action_and_where(Expr::col((S::Table, S::Value)).is_null())
                    .to_owned(),
            )
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(database.pool())
            .await?;

        let (sql, values) = Query::select()
            .column(S::Value)
            .from(S::Table)
            .and_where(Expr::col(S::Key).eq(key))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(database.pool())
            .await?)
    }

    /// Read every setting that has a value.
    pub async fn find_all(database: &Database) -> Result<HashMap<String, String>, DbError> {
        let (sql, values) = Query::select()
            .columns([S::Key, S::Value])
            .from(S::Table)
            .and_where(Expr::col(S::Value).is_not_null())
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<(String, String)> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(database.pool())
            .await?;
        Ok(rows.into_iter().collect())
    }

    /// Insert or replace a setting.
    pub async fn save(
        transaction: &mut DbTransaction,
        key: &str,
        value: &str,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(S::Table)
            .columns([S::Key, S::Value])
            .values_panic([key.into(), value.into()])
            .on_conflict(
                OnConflict::column(S::Key)
                    .update_column(S::Value)
                    .to_owned(),
            )
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    pub async fn delete(transaction: &mut DbTransaction, key: &str) -> Result<(), DbError> {
        let (sql, values) = Query::delete()
            .from_table(S::Table)
            .and_where(Expr::col(S::Key).eq(key))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }
}
