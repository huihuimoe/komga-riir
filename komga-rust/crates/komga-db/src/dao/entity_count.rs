//! The entity counts Kotlin `MetricsPublisherController` publishes as gauges.

use crate::iden::{
    BookTable, CollectionTable, ContentKey, LibraryContentTable, LibraryTable, ReadlistTable,
};
use komga_types::LibraryId;
use sea_query::{Asterisk, Expr, ExprTrait, IntoTableRef, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::connection::Database;
use crate::error::DbError;

/// Counts rows across all libraries, soft-deleted ones included, as Kotlin's repositories do.
pub struct EntityCountDao;

/// Entities counted as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotalCount {
    Libraries,
    Collections,
    ReadLists,
}

/// Values summed per library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryCount {
    Series,
    Books,
    BookFileSize,
    Sidecars,
}

impl EntityCountDao {
    pub async fn total(database: &Database, count: TotalCount) -> Result<i64, DbError> {
        let table = match count {
            TotalCount::Libraries => LibraryTable::Table.into_table_ref(),
            TotalCount::Collections => CollectionTable::Table.into_table_ref(),
            TotalCount::ReadLists => ReadlistTable::Table.into_table_ref(),
        };
        let (sql, values) = Query::select()
            .expr(Expr::col(Asterisk).count())
            .from(table)
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(database.pool())
            .await?)
    }

    /// Libraries without any row are absent.
    pub async fn per_library(
        database: &Database,
        count: LibraryCount,
    ) -> Result<Vec<(LibraryId, i64)>, DbError> {
        let (table, agg) = match count {
            LibraryCount::Series => (LibraryContentTable::Series, Expr::col(Asterisk).count()),
            LibraryCount::Books => (LibraryContentTable::Book, Expr::col(Asterisk).count()),
            LibraryCount::BookFileSize => (
                LibraryContentTable::Book,
                Expr::col(BookTable::FileSize).sum(),
            ),
            LibraryCount::Sidecars => (LibraryContentTable::Sidecar, Expr::col(Asterisk).count()),
        };
        let (sql, values) = Query::select()
            .column(ContentKey::LibraryId)
            .expr(agg)
            .from(table)
            .group_by_col(ContentKey::LibraryId)
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(database.pool())
            .await?)
    }
}
