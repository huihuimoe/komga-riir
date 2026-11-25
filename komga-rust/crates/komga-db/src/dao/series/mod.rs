//! Kotlin `SeriesDao`.

mod dto;
mod metadata;

use crate::iden::SeriesTable as S;
use jiff::civil::DateTime;
use komga_types::{FileUrl, LibraryId, Series, SeriesId};
use sea_query::{Expr, ExprTrait, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::support::local_date_time::{self, SqlDateTime};
use crate::{DbError, DbTransaction};

pub use dto::SeriesDtoDao;
pub use metadata::SeriesMetadataDao;

pub struct SeriesDao;

pub(crate) const COLUMNS: [S; 8] = [
    S::Id,
    S::Name,
    S::Url,
    S::FileLastModified,
    S::LibraryId,
    S::BookCount,
    S::DeletedDate,
    S::Oneshot,
];

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) struct SeriesRow {
    id: SeriesId,
    name: String,
    url: FileUrl,
    file_last_modified: SqlDateTime,
    library_id: LibraryId,
    book_count: i32,
    deleted_date: Option<SqlDateTime>,
    /// Declared lowercase by Kotlin's `V20230724114349__oneshots.sql`.
    #[sqlx(rename = "oneshot")]
    oneshot: bool,
}

impl SeriesRow {
    pub(crate) fn into_series(self) -> Result<Series, DbError> {
        Ok(Series {
            id: self.id,
            name: self.name,
            url: self.url,
            file_last_modified: self.file_last_modified.0,
            library_id: self.library_id,
            book_count: self.book_count,
            deleted_date: self.deleted_date.map(|date| date.0),
            oneshot: self.oneshot,
        })
    }
}

impl SeriesDao {
    /// `findByIdOrNull`, including a soft-deleted series.
    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        id: &SeriesId,
    ) -> Result<Option<Series>, DbError> {
        let (sql, values) = Query::select()
            .columns(COLUMNS)
            .from(S::Table)
            .and_where(Expr::col(S::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        let row: Option<SeriesRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        row.map(SeriesRow::into_series).transpose()
    }

    /// How many of `ids` name a series, soft-deleted or not.
    pub async fn count_by_ids(
        transaction: &mut DbTransaction,
        ids: &[SeriesId],
    ) -> Result<usize, DbError> {
        let (sql, values) = Query::select()
            .expr(Expr::col(S::Id).count())
            .from(S::Table)
            .and_where(Expr::col(S::Id).is_in(ids))
            .build_sqlx(SqliteQueryBuilder);
        let count: i64 = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?;
        Ok(usize::try_from(count).expect("COUNT is never negative"))
    }

    /// `findAllByLibraryId`, including soft-deleted series.
    pub async fn find_all_by_library(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
    ) -> Result<Vec<Series>, DbError> {
        let (sql, values) = Query::select()
            .columns(COLUMNS)
            .from(S::Table)
            .and_where(Expr::col(S::LibraryId).eq(library_id))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<SeriesRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.into_iter().map(SeriesRow::into_series).collect()
    }

    /// The ID from `findNotDeletedByLibraryIdAndUrlOrNull`.
    pub async fn find_not_deleted_id_by_library_and_url(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
        url: &FileUrl,
    ) -> Result<Option<SeriesId>, DbError> {
        let (sql, values) = Query::select()
            .column(S::Id)
            .from(S::Table)
            .and_where(Expr::col(S::LibraryId).eq(library_id))
            .and_where(Expr::col(S::Url).eq(url))
            .and_where(Expr::col(S::DeletedDate).is_null())
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?)
    }

    /// `insert`: dates other than the file and deletion dates take the table defaults.
    pub async fn insert(transaction: &mut DbTransaction, series: &Series) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(S::Table)
            .columns([
                S::Id,
                S::Name,
                S::Url,
                S::FileLastModified,
                S::LibraryId,
                S::DeletedDate,
                S::Oneshot,
            ])
            .values_panic([
                (&series.id).into(),
                series.name.as_str().into(),
                (&series.url).into(),
                SqlDateTime(series.file_last_modified).into(),
                (&series.library_id).into(),
                series.deleted_date.map(SqlDateTime).into(),
                series.oneshot.into(),
            ])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `update(series, updateModifiedTime = true)` for the file and deletion dates the scan
    /// changes.
    pub async fn update_file_state(
        transaction: &mut DbTransaction,
        id: &SeriesId,
        file_last_modified: DateTime,
        deleted_date: Option<DateTime>,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(S::Table)
            .value(S::FileLastModified, SqlDateTime(file_last_modified))
            .value(S::DeletedDate, deleted_date.map(SqlDateTime))
            .value(S::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(S::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `update(series)` for a oneshot series whose book file was replaced.
    pub async fn update_file(
        transaction: &mut DbTransaction,
        id: &SeriesId,
        url: &FileUrl,
        file_last_modified: DateTime,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(S::Table)
            .value(S::Url, url)
            .value(S::FileLastModified, SqlDateTime(file_last_modified))
            .value(S::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(S::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `update(series, updateModifiedTime = false)` after `sortBooks` counts the books.
    pub async fn update_book_count(
        transaction: &mut DbTransaction,
        id: &SeriesId,
        book_count: i32,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(S::Table)
            .value(S::BookCount, book_count)
            .and_where(Expr::col(S::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }
}
