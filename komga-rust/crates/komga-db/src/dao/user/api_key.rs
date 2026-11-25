//! API-key persistence for `KomgaUserDao`, split from `user` by table family.

use crate::iden::UserApiKeyTable;
use komga_types::{ApiKey, KomgaUser};
use sea_query::{Expr, ExprTrait, Func, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::{AssertSqlSafe, Row};

use super::KomgaUserDao;
use crate::connection::{Database, DbTransaction};
use crate::error::DbError;

impl KomgaUserDao {
    /// Check if an API key comment already exists for a user within a transaction.
    pub async fn exists_api_key_by_comment_and_user_id(
        transaction: &mut DbTransaction,
        comment: &str,
        user_id: &str,
    ) -> Result<bool, DbError> {
        let (sql, values) = Query::select()
            .expr(Func::count(Expr::col(UserApiKeyTable::Id)))
            .from(UserApiKeyTable::Table)
            .and_where(Expr::col(UserApiKeyTable::UserId).eq(user_id))
            .and_where(Expr::col(UserApiKeyTable::Comment).eq(comment))
            .build_sqlx(SqliteQueryBuilder);

        let count: i64 = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?;
        Ok(count > 0)
    }

    /// Insert a new hashed API key record.
    pub async fn insert_api_key(
        transaction: &mut DbTransaction,
        id: &str,
        user_id: &str,
        key_hash: &str,
        comment: &str,
    ) -> Result<ApiKey, DbError> {
        let (sql, values) = Query::insert()
            .into_table(UserApiKeyTable::Table)
            .columns([
                UserApiKeyTable::Id,
                UserApiKeyTable::UserId,
                UserApiKeyTable::ApiKey,
                UserApiKeyTable::Comment,
            ])
            .values_panic([id.into(), user_id.into(), key_hash.into(), comment.into()])
            .build_sqlx(SqliteQueryBuilder);

        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;

        let (sql, values) = Query::select()
            .columns([
                UserApiKeyTable::Id,
                UserApiKeyTable::UserId,
                UserApiKeyTable::CreatedDate,
                UserApiKeyTable::LastModifiedDate,
                UserApiKeyTable::ApiKey,
                UserApiKeyTable::Comment,
            ])
            .from(UserApiKeyTable::Table)
            .and_where(Expr::col(UserApiKeyTable::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);

        let row = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?;
        Ok(api_key_from_row(&row)?)
    }

    /// Find user and API key by hashed API key token.
    pub async fn find_user_by_api_key_hash(
        database: &Database,
        api_key_hash: &str,
    ) -> Result<Option<(KomgaUser, ApiKey)>, DbError> {
        let pool = database.pool();
        let (sql, values) = Query::select()
            .columns([
                (UserApiKeyTable::Table, UserApiKeyTable::Id),
                (UserApiKeyTable::Table, UserApiKeyTable::UserId),
                (UserApiKeyTable::Table, UserApiKeyTable::CreatedDate),
                (UserApiKeyTable::Table, UserApiKeyTable::LastModifiedDate),
                (UserApiKeyTable::Table, UserApiKeyTable::ApiKey),
                (UserApiKeyTable::Table, UserApiKeyTable::Comment),
            ])
            .from(UserApiKeyTable::Table)
            .and_where(Expr::col(UserApiKeyTable::ApiKey).eq(api_key_hash))
            .build_sqlx(SqliteQueryBuilder);

        let row = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(pool)
            .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let api_key = api_key_from_row(&row)?;

        let Some(user) = Self::find_by_id(database, api_key.user_id.as_str()).await? else {
            return Ok(None);
        };

        Ok(Some((user, api_key)))
    }

    /// List all API keys for a user.
    pub async fn find_api_keys_by_user_id(
        database: &Database,
        user_id: &str,
    ) -> Result<Vec<ApiKey>, DbError> {
        let (sql, values) = Query::select()
            .columns([
                UserApiKeyTable::Id,
                UserApiKeyTable::UserId,
                UserApiKeyTable::CreatedDate,
                UserApiKeyTable::LastModifiedDate,
                UserApiKeyTable::ApiKey,
                UserApiKeyTable::Comment,
            ])
            .from(UserApiKeyTable::Table)
            .and_where(Expr::col(UserApiKeyTable::UserId).eq(user_id))
            .order_by(UserApiKeyTable::Comment, sea_query::Order::Asc)
            .build_sqlx(SqliteQueryBuilder);

        let rows = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(database.pool())
            .await?;

        rows.iter()
            .map(api_key_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// Delete an API key by ID and user ID.
    pub async fn delete_api_key_by_id(
        transaction: &mut DbTransaction,
        key_id: &str,
        user_id: &str,
    ) -> Result<bool, DbError> {
        let (sql, values) = Query::delete()
            .from_table(UserApiKeyTable::Table)
            .and_where(Expr::col(UserApiKeyTable::Id).eq(key_id))
            .and_where(Expr::col(UserApiKeyTable::UserId).eq(user_id))
            .build_sqlx(SqliteQueryBuilder);

        let result = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;

        Ok(result.rows_affected() > 0)
    }
}

fn api_key_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ApiKey, sqlx::Error> {
    Ok(ApiKey {
        id: row.try_get("ID")?,
        user_id: row.try_get("USER_ID")?,
        created_date: row.try_get("CREATED_DATE")?,
        last_modified_date: row.try_get("LAST_MODIFIED_DATE")?,
        key: row.try_get("API_KEY")?,
        comment: row.try_get("COMMENT")?,
    })
}
