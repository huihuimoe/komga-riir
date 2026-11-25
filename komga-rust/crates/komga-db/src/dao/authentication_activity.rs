//! DAO for authentication activity records.

use crate::iden::{AuthenticationActivityTable as AA, UserTable};
use jiff::civil::DateTime;
use komga_types::{
    ActivitySort, AuthenticationActivity, KomgaUser, PageRequest, PageResult, RecordActivityParams,
    SortOrder,
};
use sea_query::{
    Asterisk, Condition, Expr, ExprTrait, Order, Query, SelectStatement, SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::sqlite::SqliteRow;
use sqlx::{AssertSqlSafe, Row};

use crate::connection::{Database, DbTransaction};
use crate::error::DbError;
use crate::support::local_date_time::{self, SqlDateTime};
use crate::support::rows::fetch_page;

/// DAO for `AUTHENTICATION_ACTIVITY` table.
pub struct AuthenticationActivityDao;

impl AuthenticationActivityDao {
    /// Insert an authentication activity record, stamped now in UTC with sub-second precision
    /// so that same-second attempts keep their order (the column default has seconds only).
    pub async fn insert(
        transaction: &mut DbTransaction,
        activity: &RecordActivityParams<'_>,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(AA::Table)
            .columns([
                AA::UserId,
                AA::Email,
                AA::Ip,
                AA::UserAgent,
                AA::Success,
                AA::Error,
                AA::Source,
                AA::ApiKeyId,
                AA::ApiKeyComment,
                AA::DateTime,
            ])
            .values_panic([
                activity.user_id.into(),
                activity.email.into(),
                activity.ip.into(),
                activity.user_agent.into(),
                activity.success.into(),
                activity.error.into(),
                activity.source.into(),
                activity.api_key_id.into(),
                activity.api_key_comment.into(),
                local_date_time::now_utc().into(),
            ])
            .build_sqlx(SqliteQueryBuilder);

        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// The requested page of all activity, or of `user`'s (by id or email).
    pub async fn find_page(
        database: &Database,
        user: Option<&KomgaUser>,
        sort: &[SortOrder<ActivitySort>],
        page: PageRequest,
    ) -> Result<PageResult<AuthenticationActivity>, DbError> {
        let mut query = select();
        if let Some(user) = user {
            query.cond_where(of_user(user));
        }
        let orders = sort
            .iter()
            .map(|order| (Expr::col(sort_column(order.key)), order.direction));
        fetch_page(database, query, orders, page, from_row).await
    }

    /// `user`'s most recent activity, with `api_key_id` only that key's.
    pub async fn find_latest(
        database: &Database,
        user: &KomgaUser,
        api_key_id: Option<&str>,
    ) -> Result<Option<AuthenticationActivity>, DbError> {
        let mut query = select();
        query.cond_where(of_user(user));
        if let Some(api_key_id) = api_key_id {
            query.and_where(Expr::col(AA::ApiKeyId).eq(api_key_id));
        }
        let (sql, values) = query
            .order_by(AA::DateTime, Order::Desc)
            .limit(1)
            .build_sqlx(SqliteQueryBuilder);
        let row = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(database.pool())
            .await?;
        row.as_ref().map(from_row).transpose()
    }

    /// Deletes the activity of a user by id or by its current email, as `deleteByUser`.
    pub async fn delete_by_user(
        transaction: &mut DbTransaction,
        user_id: &str,
    ) -> Result<(), DbError> {
        let email = Query::select()
            .column(UserTable::Email)
            .from(UserTable::Table)
            .and_where(Expr::col(UserTable::Id).eq(user_id))
            .take();
        let (sql, values) = Query::delete()
            .from_table(AA::Table)
            .cond_where(
                Condition::any()
                    .add(Expr::col(AA::UserId).eq(user_id))
                    .add(Expr::col(AA::Email).in_subquery(email)),
            )
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Deletes the activity recorded before `cutoff` (UTC) and returns how much.
    pub async fn delete_older_than(
        transaction: &mut DbTransaction,
        cutoff: DateTime,
    ) -> Result<u64, DbError> {
        let (sql, values) = Query::delete()
            .from_table(AA::Table)
            .and_where(Expr::col(AA::DateTime).lt(SqlDateTime(cutoff)))
            .build_sqlx(SqliteQueryBuilder);
        let result = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(result.rows_affected())
    }
}

fn select() -> SelectStatement {
    let mut query = Query::select();
    query.column(Asterisk).from(AA::Table);
    query
}

fn of_user(user: &KomgaUser) -> Condition {
    Condition::any()
        .add(Expr::col(AA::UserId).eq(user.id.as_str()))
        .add(Expr::col(AA::Email).eq(user.email.as_str()))
}

fn sort_column(key: ActivitySort) -> AA {
    match key {
        ActivitySort::DateTime => AA::DateTime,
        ActivitySort::Email => AA::Email,
        ActivitySort::Success => AA::Success,
        ActivitySort::Ip => AA::Ip,
        ActivitySort::Error => AA::Error,
        ActivitySort::UserId => AA::UserId,
        ActivitySort::UserAgent => AA::UserAgent,
    }
}

fn from_row(row: &SqliteRow) -> Result<AuthenticationActivity, DbError> {
    Ok(AuthenticationActivity {
        user_id: row.try_get("USER_ID")?,
        email: row.try_get("EMAIL")?,
        ip: row.try_get("IP")?,
        user_agent: row.try_get("USER_AGENT")?,
        success: row.try_get("SUCCESS")?,
        error: row.try_get("ERROR")?,
        source: row.try_get("SOURCE")?,
        api_key_id: row.try_get("API_KEY_ID")?,
        api_key_comment: row.try_get("API_KEY_COMMENT")?,
        date_time: row.try_get::<SqlDateTime, _>("DATE_TIME")?.0,
    })
}
