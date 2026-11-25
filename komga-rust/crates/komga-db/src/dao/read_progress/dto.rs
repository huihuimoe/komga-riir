//! Kotlin `ReadProgressDtoDao`: per-book completion behind the Mihon read progress views.

use std::collections::HashMap;

use crate::iden::BookMetadataTable as D;
use crate::iden::BookTable as B;
use crate::iden::ReadProgressTable as R;
use komga_types::{BookId, SeriesId, UserId};
use sea_query::{Expr, ExprTrait, Order, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::{DbError, DbTransaction};

pub struct ReadProgressDtoDao;

/// A book of a series and whether the user completed it: `None` unread, `Some(false)` started.
#[derive(Debug, Clone, Copy, PartialEq, sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct NumberedCompletion {
    pub number_sort: f32,
    pub completed: Option<bool>,
}

impl ReadProgressDtoDao {
    /// The books of the series, soft-deleted ones included, by `NUMBER_SORT`, as
    /// `findProgressV2BySeries` reads them.
    pub async fn find_series_completion(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
        user_id: &UserId,
    ) -> Result<Vec<NumberedCompletion>, DbError> {
        let (sql, values) = Query::select()
            .column((D::Table, D::NumberSort))
            .column((R::Table, R::Completed))
            .from(B::Table)
            .inner_join(
                D::Table,
                Expr::col((D::Table, D::BookId)).equals((B::Table, B::Id)),
            )
            .left_join(
                R::Table,
                Expr::col((R::Table, R::BookId))
                    .equals((B::Table, B::Id))
                    .and(Expr::col((R::Table, R::UserId)).eq(user_id)),
            )
            .and_where(Expr::col((B::Table, B::SeriesId)).eq(series_id))
            .order_by((D::Table, D::NumberSort), Order::Asc)
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }

    /// Whether the user completed each of the books, in their order: `None` unread,
    /// `Some(false)` started.
    pub async fn find_completion(
        transaction: &mut DbTransaction,
        book_ids: &[BookId],
        user_id: &UserId,
    ) -> Result<Vec<Option<bool>>, DbError> {
        let (sql, values) = Query::select()
            .columns([R::BookId, R::Completed])
            .from(R::Table)
            .and_where(Expr::col(R::UserId).eq(user_id))
            .and_where(Expr::col(R::BookId).is_in(book_ids))
            .build_sqlx(SqliteQueryBuilder);
        let completed: HashMap<BookId, bool> =
            sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?
                .into_iter()
                .collect();
        Ok(book_ids
            .iter()
            .map(|id| completed.get(id).copied())
            .collect())
    }
}
