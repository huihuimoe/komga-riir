//! DAO for `ANNOUNCEMENTS_READ`: the announcements each user marked read.

use std::collections::BTreeSet;

use crate::iden::AnnouncementsReadTable as AR;
use sea_query::{Expr, ExprTrait, OnConflict, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::connection::{Database, DbTransaction};
use crate::error::DbError;

pub struct AnnouncementsReadDao;

impl AnnouncementsReadDao {
    pub async fn find_ids(database: &Database, user_id: &str) -> Result<BTreeSet<String>, DbError> {
        let (sql, values) = Query::select()
            .column(AR::AnnouncementId)
            .from(AR::Table)
            .and_where(Expr::col(AR::UserId).eq(user_id))
            .build_sqlx(SqliteQueryBuilder);
        let ids = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(database.pool())
            .await?;
        Ok(ids.into_iter().collect())
    }

    /// Marks `ids` read by the user; those already read are left alone.
    pub async fn insert(
        transaction: &mut DbTransaction,
        user_id: &str,
        ids: &BTreeSet<String>,
    ) -> Result<(), DbError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut insert = Query::insert();
        insert
            .into_table(AR::Table)
            .columns([AR::UserId, AR::AnnouncementId])
            .on_conflict(OnConflict::new().do_nothing().to_owned());
        for id in ids {
            insert.values_panic([user_id.into(), id.as_str().into()]);
        }
        let (sql, values) = insert.build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }
}
