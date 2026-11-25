//! Kotlin `ThumbnailSeriesDao`.

use crate::iden::ThumbnailSeriesTable as T;
use komga_types::{FileUrl, SeriesId, ThumbnailDetails, ThumbnailId, ThumbnailType};
use sea_query::{Expr, ExprTrait, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::{ContentRow, DetailsRow, EntryRow, NewThumbnail, ThumbnailContent, ThumbnailEntry};
use crate::{DbError, DbTransaction};

/// Bound parameters per `DELETE`, well below SQLite's variable limit.
const DELETE_CHUNK: usize = 1000;

pub struct ThumbnailSeriesDao;

impl ThumbnailSeriesDao {
    /// `findAllBySeriesId`, in table order.
    pub async fn find_all_by_series(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<Vec<ThumbnailEntry>, DbError> {
        let (sql, values) = Query::select()
            .columns([T::Id, T::Type, T::Selected])
            .expr_as(
                Expr::col(T::Thumbnail)
                    .is_not_null()
                    .or(Expr::col(T::Url).is_not_null()),
                "STORED",
            )
            .from(T::Table)
            .and_where(Expr::col(T::SeriesId).eq(series_id))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<EntryRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.into_iter().map(EntryRow::into_entry).collect()
    }

    /// `findAllBySeriesId` without the bytes, in table order.
    pub async fn find_all_details_by_series(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<Vec<ThumbnailDetails>, DbError> {
        let (sql, values) = Query::select()
            .columns([
                T::Id,
                T::Type,
                T::Selected,
                T::MediaType,
                T::FileSize,
                T::Width,
                T::Height,
            ])
            .from(T::Table)
            .and_where(Expr::col(T::SeriesId).eq(series_id))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<DetailsRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.into_iter().map(DetailsRow::into_details).collect()
    }

    /// The series and type of a thumbnail.
    pub async fn find_owner(
        transaction: &mut DbTransaction,
        id: &ThumbnailId,
    ) -> Result<Option<(SeriesId, ThumbnailType)>, DbError> {
        let (sql, values) = Query::select()
            .columns([T::SeriesId, T::Type])
            .from(T::Table)
            .and_where(Expr::col(T::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?)
    }

    /// `findByIdOrNull`, for its content.
    pub async fn find_content(
        transaction: &mut DbTransaction,
        id: &ThumbnailId,
    ) -> Result<Option<ThumbnailContent>, DbError> {
        let (sql, values) = Query::select()
            .columns([T::Id, T::Type, T::Thumbnail, T::Url, T::MediaType])
            .from(T::Table)
            .and_where(Expr::col(T::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        let row: Option<ContentRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        row.map(ContentRow::into_content).transpose()
    }

    /// `findSelectedBySeriesIdOrNull`: the first selected row.
    pub async fn find_selected(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<Option<ThumbnailContent>, DbError> {
        let (sql, values) = Query::select()
            .columns([T::Id, T::Type, T::Thumbnail, T::Url, T::MediaType])
            .from(T::Table)
            .and_where(Expr::col(T::SeriesId).eq(series_id))
            .and_where(Expr::col(T::Selected).eq(true))
            .limit(1)
            .build_sqlx(SqliteQueryBuilder);
        let row: Option<ContentRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        row.map(ContentRow::into_content).transpose()
    }

    pub async fn insert(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
        thumbnail: &NewThumbnail<'_>,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(T::Table)
            .columns([
                T::Id,
                T::SeriesId,
                T::Thumbnail,
                T::Url,
                T::Selected,
                T::Type,
                T::MediaType,
                T::Width,
                T::Height,
                T::FileSize,
            ])
            .values_panic([
                thumbnail.id.into(),
                series_id.into(),
                thumbnail.thumbnail.map(<[u8]>::to_vec).into(),
                thumbnail.url.map(FileUrl::as_str).into(),
                false.into(),
                thumbnail.kind.into(),
                thumbnail.media_type.into(),
                thumbnail.dimension.width.into(),
                thumbnail.dimension.height.into(),
                thumbnail.file_size.into(),
            ])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Stores the bytes of a row that only had a URL; a row with bytes is left alone.
    pub async fn fill_thumbnail(
        transaction: &mut DbTransaction,
        id: &ThumbnailId,
        bytes: &[u8],
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(T::Table)
            .value(T::Thumbnail, bytes.to_vec())
            .and_where(Expr::col(T::Id).eq(id))
            .and_where(Expr::col(T::Thumbnail).is_null())
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `markSelected`: selects this row and unselects the series' others.
    pub async fn mark_selected(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
        id: &ThumbnailId,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(T::Table)
            .value(T::Selected, Expr::col(T::Id).eq(id))
            .and_where(Expr::col(T::SeriesId).eq(series_id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Returns whether the deleted thumbnail was the selected one.
    pub async fn delete(
        transaction: &mut DbTransaction,
        id: &ThumbnailId,
    ) -> Result<bool, DbError> {
        let (sql, values) = Query::delete()
            .from_table(T::Table)
            .and_where(Expr::col(T::Id).eq(id))
            .returning_col(T::Selected)
            .build_sqlx(SqliteQueryBuilder);
        let selected: Option<bool> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        Ok(selected.unwrap_or(false))
    }

    /// The files of the series' thumbnails of type `kind` read from one.
    pub async fn find_urls_by_series_and_type(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
        kind: ThumbnailType,
    ) -> Result<Vec<FileUrl>, DbError> {
        let (sql, values) = Query::select()
            .column(T::Url)
            .from(T::Table)
            .and_where(Expr::col(T::SeriesId).eq(series_id))
            .and_where(Expr::col(T::Type).eq(kind.name()))
            .and_where(Expr::col(T::Url).is_not_null())
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }

    /// Deletes the series' thumbnails of one type, like `ThumbnailBookDao.deleteByBookIdAndType`.
    pub async fn delete_by_series_and_type(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
        kind: ThumbnailType,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::delete()
            .from_table(T::Table)
            .and_where(Expr::col(T::SeriesId).eq(series_id))
            .and_where(Expr::col(T::Type).eq(kind.name()))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Deletes the sidecar thumbnails read from `urls`, returning the series they belonged to.
    pub async fn delete_sidecars_by_urls(
        transaction: &mut DbTransaction,
        urls: &[&FileUrl],
    ) -> Result<Vec<SeriesId>, DbError> {
        let mut series_ids = Vec::new();
        for chunk in urls.chunks(DELETE_CHUNK) {
            let (sql, values) = Query::delete()
                .from_table(T::Table)
                .and_where(Expr::col(T::Type).eq(ThumbnailType::Sidecar.name()))
                .and_where(Expr::col(T::Url).is_in(chunk.iter().copied()))
                .returning_col(T::SeriesId)
                .build_sqlx(SqliteQueryBuilder);
            let ids: Vec<SeriesId> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?;
            series_ids.extend(ids);
        }
        Ok(series_ids)
    }
}
