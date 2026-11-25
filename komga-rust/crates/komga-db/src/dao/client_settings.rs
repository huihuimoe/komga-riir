//! DAO for client settings (Kotlin `ClientSettingsDtoDao`).

use std::collections::BTreeMap;

use crate::iden::{ClientSettingsGlobalTable, ClientSettingsUserTable};
use komga_types::GlobalClientSetting;
use sea_query::{Expr, ExprTrait, OnConflict, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::connection::{Database, DbTransaction};
use crate::error::DbError;

/// Read and write `CLIENT_SETTINGS_GLOBAL` and `CLIENT_SETTINGS_USER`.
pub struct ClientSettingsDao;

impl ClientSettingsDao {
    /// Read global settings, only those allowing unauthorized access when `only_unauthorized`.
    pub async fn find_all_global(
        database: &Database,
        only_unauthorized: bool,
    ) -> Result<BTreeMap<String, GlobalClientSetting>, DbError> {
        let mut query = Query::select();
        query
            .columns([
                ClientSettingsGlobalTable::Key,
                ClientSettingsGlobalTable::Value,
                ClientSettingsGlobalTable::AllowUnauthorized,
            ])
            .from(ClientSettingsGlobalTable::Table);
        if only_unauthorized {
            query.and_where(Expr::col(ClientSettingsGlobalTable::AllowUnauthorized).eq(true));
        }
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let rows: Vec<(String, String, bool)> =
            sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(database.read_pool())
                .await?;
        Ok(rows
            .into_iter()
            .map(|(key, value, allow_unauthorized)| {
                (
                    key,
                    GlobalClientSetting {
                        value,
                        allow_unauthorized,
                    },
                )
            })
            .collect())
    }

    /// Read the settings of one user as key/value pairs.
    pub async fn find_all_user(
        database: &Database,
        user_id: &str,
    ) -> Result<BTreeMap<String, String>, DbError> {
        let (sql, values) = Query::select()
            .columns([ClientSettingsUserTable::Key, ClientSettingsUserTable::Value])
            .from(ClientSettingsUserTable::Table)
            .and_where(Expr::col(ClientSettingsUserTable::UserId).eq(user_id))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<(String, String)> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(database.read_pool())
            .await?;
        Ok(rows.into_iter().collect())
    }

    /// Insert or replace a global setting, including its `ALLOW_UNAUTHORIZED` flag.
    pub async fn save_global(
        transaction: &mut DbTransaction,
        key: &str,
        setting: &GlobalClientSetting,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(ClientSettingsGlobalTable::Table)
            .columns([
                ClientSettingsGlobalTable::Key,
                ClientSettingsGlobalTable::Value,
                ClientSettingsGlobalTable::AllowUnauthorized,
            ])
            .values_panic([
                key.into(),
                setting.value.as_str().into(),
                setting.allow_unauthorized.into(),
            ])
            .on_conflict(
                OnConflict::column(ClientSettingsGlobalTable::Key)
                    .update_columns([
                        ClientSettingsGlobalTable::Value,
                        ClientSettingsGlobalTable::AllowUnauthorized,
                    ])
                    .to_owned(),
            )
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Insert or replace one user setting.
    pub async fn save_for_user(
        transaction: &mut DbTransaction,
        user_id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(ClientSettingsUserTable::Table)
            .columns([
                ClientSettingsUserTable::UserId,
                ClientSettingsUserTable::Key,
                ClientSettingsUserTable::Value,
            ])
            .values_panic([user_id.into(), key.into(), value.into()])
            .on_conflict(
                OnConflict::columns([
                    ClientSettingsUserTable::Key,
                    ClientSettingsUserTable::UserId,
                ])
                .update_column(ClientSettingsUserTable::Value)
                .to_owned(),
            )
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    pub async fn delete_global_by_keys(
        database: &Database,
        keys: &[String],
    ) -> Result<(), DbError> {
        let (sql, values) = Query::delete()
            .from_table(ClientSettingsGlobalTable::Table)
            .and_where(Expr::col(ClientSettingsGlobalTable::Key).is_in(keys))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(database.write_pool())
            .await?;
        Ok(())
    }

    pub async fn delete_by_user_id_and_keys(
        database: &Database,
        user_id: &str,
        keys: &[String],
    ) -> Result<(), DbError> {
        let (sql, values) = Query::delete()
            .from_table(ClientSettingsUserTable::Table)
            .and_where(Expr::col(ClientSettingsUserTable::Key).is_in(keys))
            .and_where(Expr::col(ClientSettingsUserTable::UserId).eq(user_id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(database.write_pool())
            .await?;
        Ok(())
    }
}
