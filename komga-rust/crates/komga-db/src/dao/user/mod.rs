//! DAO for Komga users and their lifecycle-owned records.

mod api_key;
mod relations;

use crate::iden::{
    AnnouncementsReadTable, ClientSettingsUserTable, ReadProgressSeriesTable, ReadProgressTable,
    UserApiKeyTable, UserLibrarySharingTable, UserRoleTable, UserSharingTable, UserTable,
};
use komga_types::{AgeRestriction, AllowExclude, ContentRestrictions, KomgaUser, UserId};
use sea_query::{Alias, Expr, ExprTrait, Func, Iden, Query, SelectStatement, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::sqlite::SqliteRow;
use sqlx::{AssertSqlSafe, Row, SqlitePool};

use crate::connection::{Database, DbTransaction};
use crate::dao::{AuthenticationActivityDao, SyncPointDao};
use crate::error::DbError;

pub struct KomgaUserDao;

impl KomgaUserDao {
    pub async fn count(database: &Database) -> Result<i64, DbError> {
        let pool = database.read_pool();
        let (sql, values) = Query::select()
            .expr(Func::count(Expr::col(UserTable::Id)))
            .from(UserTable::Table)
            .build_sqlx(SqliteQueryBuilder);

        let count: i64 = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(pool)
            .await?;
        Ok(count)
    }

    pub async fn count_in_transaction(transaction: &mut DbTransaction) -> Result<i64, DbError> {
        let (sql, values) = Query::select()
            .expr(Func::count(Expr::col(UserTable::Id)))
            .from(UserTable::Table)
            .build_sqlx(SqliteQueryBuilder);

        let count: i64 = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?;
        Ok(count)
    }

    pub async fn exists_by_email_ignore_case(
        transaction: &mut DbTransaction,
        email: &str,
    ) -> Result<bool, DbError> {
        let (sql, values) = Query::select()
            .expr(Func::count(Expr::col(UserTable::Id)))
            .from(UserTable::Table)
            .and_where(Expr::expr(Func::lower(Expr::col(UserTable::Email))).eq(Func::lower(email)))
            .build_sqlx(SqliteQueryBuilder);

        let count: i64 = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?;
        Ok(count > 0)
    }

    pub async fn find_by_id(database: &Database, id: &str) -> Result<Option<KomgaUser>, DbError> {
        let pool = database.read_pool();
        let (sql, values) = Self::select_users()
            .and_where(Expr::col(UserTable::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);

        let row = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(pool)
            .await?;
        match row {
            Some(row) => Ok(Self::users_from_rows(pool, &[row]).await?.pop()),
            None => Ok(None),
        }
    }

    pub async fn find_by_email_ignore_case(
        database: &Database,
        email: &str,
    ) -> Result<Option<KomgaUser>, DbError> {
        let pool = database.read_pool();
        let (sql, values) = Query::select()
            .column(UserTable::Id)
            .from(UserTable::Table)
            .and_where(Expr::expr(Func::lower(Expr::col(UserTable::Email))).eq(Func::lower(email)))
            .build_sqlx(SqliteQueryBuilder);

        let row = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(pool)
            .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let user_id: String = row.try_get("ID")?;
        Self::find_by_id(database, &user_id).await
    }

    /// Find all users ordered by email.
    pub async fn find_all(database: &Database) -> Result<Vec<KomgaUser>, DbError> {
        let pool = database.read_pool();
        let (sql, values) = Self::select_users()
            .order_by(UserTable::Email, sea_query::Order::Asc)
            .build_sqlx(SqliteQueryBuilder);

        let rows = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(pool)
            .await?;

        Ok(Self::users_from_rows(pool, &rows).await?)
    }

    fn select_users() -> SelectStatement {
        Query::select()
            .columns([
                UserTable::Id,
                UserTable::Email,
                UserTable::Password,
                UserTable::SharedAllLibraries,
                UserTable::AgeRestriction,
                UserTable::AgeRestrictionAllowOnly,
            ])
            .from(UserTable::Table)
            .to_owned()
    }

    async fn users_from_rows(
        pool: &SqlitePool,
        rows: &[SqliteRow],
    ) -> Result<Vec<KomgaUser>, sqlx::Error> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let mut user_ids = Vec::with_capacity(rows.len());
        for row in rows {
            user_ids.push(row.try_get::<UserId, _>("ID")?);
        }
        let mut roles_by_user = relations::find_roles_by_user_ids(pool, &user_ids).await?;
        let mut libraries_by_user =
            relations::find_shared_library_ids_by_user_ids(pool, &user_ids).await?;
        let mut labels_by_user =
            relations::find_sharing_labels_by_user_ids(pool, &user_ids).await?;

        let mut users = Vec::with_capacity(rows.len());
        for (row, user_id) in rows.iter().zip(user_ids) {
            let age: Option<i32> = row.try_get("AGE_RESTRICTION")?;
            let allow_only: Option<bool> = row.try_get("AGE_RESTRICTION_ALLOW_ONLY")?;
            let age_restriction = age.zip(allow_only).map(|(age, allow_only)| AgeRestriction {
                age,
                restriction: if allow_only {
                    AllowExclude::AllowOnly
                } else {
                    AllowExclude::Exclude
                },
            });
            let (labels_allow, labels_exclude) =
                labels_by_user.remove(&user_id).unwrap_or_default();
            let roles = roles_by_user.remove(&user_id).unwrap_or_default();
            let shared_library_ids = libraries_by_user.remove(&user_id).unwrap_or_default();

            users.push(KomgaUser {
                email: row.try_get("EMAIL")?,
                password_hash: row.try_get("PASSWORD")?,
                roles,
                shared_all_libraries: row.try_get("SHARED_ALL_LIBRARIES")?,
                shared_library_ids,
                restrictions: ContentRestrictions::new(
                    age_restriction,
                    labels_allow,
                    labels_exclude,
                ),
                id: user_id,
            });
        }
        Ok(users)
    }

    pub async fn insert(transaction: &mut DbTransaction, user: &KomgaUser) -> Result<(), DbError> {
        let (age, allow_only) = age_restriction_columns(&user.restrictions);
        let (sql, values) = Query::insert()
            .into_table(UserTable::Table)
            .columns([
                UserTable::Id,
                UserTable::Email,
                UserTable::Password,
                UserTable::SharedAllLibraries,
                UserTable::AgeRestriction,
                UserTable::AgeRestrictionAllowOnly,
            ])
            .values_panic([
                (&user.id).into(),
                user.email.clone().into(),
                user.password_hash.clone().into(),
                user.shared_all_libraries.into(),
                age.into(),
                allow_only.into(),
            ])
            .build_sqlx(SqliteQueryBuilder);

        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;

        relations::insert_user_relations(transaction.connection(), user).await?;
        Ok(())
    }

    /// Update user fields and replace roles, shared libraries, and sharing labels.
    pub async fn update(transaction: &mut DbTransaction, user: &KomgaUser) -> Result<(), DbError> {
        let (age, allow_only) = age_restriction_columns(&user.restrictions);
        let (sql, values) = Query::update()
            .table(UserTable::Table)
            .values([
                (UserTable::Email, user.email.clone().into()),
                (
                    UserTable::SharedAllLibraries,
                    user.shared_all_libraries.into(),
                ),
                (UserTable::AgeRestriction, age.into()),
                (UserTable::AgeRestrictionAllowOnly, allow_only.into()),
            ])
            .value(UserTable::LastModifiedDate, Expr::current_timestamp())
            .and_where(Expr::col(UserTable::Id).eq(&user.id))
            .build_sqlx(SqliteQueryBuilder);

        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;

        Self::delete_by_user_id(transaction, UserRoleTable::Table, &user.id).await?;
        Self::delete_by_user_id(transaction, UserLibrarySharingTable::Table, &user.id).await?;
        Self::delete_by_user_id(transaction, UserSharingTable::Table, &user.id).await?;
        relations::insert_user_relations(transaction.connection(), user).await?;
        Ok(())
    }

    pub async fn update_password(
        transaction: &mut DbTransaction,
        user_id: &str,
        new_password_hash: &str,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(UserTable::Table)
            .values([(UserTable::Password, new_password_hash.into())])
            .and_where(Expr::col(UserTable::Id).eq(user_id))
            .build_sqlx(SqliteQueryBuilder);

        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Delete a user and all records that reference the user, sync points included.
    pub async fn delete(transaction: &mut DbTransaction, user_id: &str) -> Result<bool, DbError> {
        SyncPointDao::delete_by_user(transaction, &UserId::from(user_id), None).await?;
        Self::delete_by_user_id(transaction, ClientSettingsUserTable::Table, user_id).await?;
        Self::delete_by_user_id(transaction, ReadProgressTable::Table, user_id).await?;
        Self::delete_by_user_id(transaction, ReadProgressSeriesTable::Table, user_id).await?;
        AuthenticationActivityDao::delete_by_user(transaction, user_id).await?;
        Self::delete_by_user_id(transaction, UserApiKeyTable::Table, user_id).await?;
        Self::delete_by_user_id(transaction, AnnouncementsReadTable::Table, user_id).await?;
        Self::delete_by_user_id(transaction, UserSharingTable::Table, user_id).await?;
        Self::delete_by_user_id(transaction, UserLibrarySharingTable::Table, user_id).await?;
        Self::delete_by_user_id(transaction, UserRoleTable::Table, user_id).await?;

        let (del_user_sql, del_user_values) = Query::delete()
            .from_table(UserTable::Table)
            .and_where(Expr::col(UserTable::Id).eq(user_id))
            .build_sqlx(SqliteQueryBuilder);

        let result = sqlx::query_with(AssertSqlSafe(del_user_sql.as_str()), del_user_values)
            .execute(transaction.connection())
            .await?;

        Ok(result.rows_affected() > 0)
    }

    async fn delete_by_user_id<T: Iden>(
        transaction: &mut DbTransaction,
        table: T,
        user_id: impl Into<sea_query::Value>,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::delete()
            .from_table(table)
            .and_where(Expr::col(Alias::new("USER_ID")).eq(user_id))
            .build_sqlx(SqliteQueryBuilder);

        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }
}

/// `(AGE_RESTRICTION, AGE_RESTRICTION_ALLOW_ONLY)` column values; both NULL without a restriction.
fn age_restriction_columns(restrictions: &ContentRestrictions) -> (Option<i32>, Option<bool>) {
    restrictions
        .age_restriction()
        .map(|restriction| {
            (
                Some(restriction.age),
                Some(restriction.restriction == AllowExclude::AllowOnly),
            )
        })
        .unwrap_or_default()
}
