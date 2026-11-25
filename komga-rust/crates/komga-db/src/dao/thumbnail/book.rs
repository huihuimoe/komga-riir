//! Kotlin `ThumbnailBookDao`.

use crate::iden::ThumbnailBookTable as T;
use komga_types::{BookId, FileUrl, ThumbnailDetails, ThumbnailId, ThumbnailType};
use sea_query::{Expr, ExprTrait, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::{ContentRow, DetailsRow, EntryRow, NewThumbnail, ThumbnailContent, ThumbnailEntry};
use crate::{DbError, DbTransaction};

/// Bound parameters per `DELETE`, well below SQLite's variable limit.
const DELETE_CHUNK: usize = 1000;

pub struct ThumbnailBookDao;

impl ThumbnailBookDao {
    /// `findAllByBookId`, in table order.
    pub async fn find_all_by_book(
        transaction: &mut DbTransaction,
        book_id: &BookId,
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
            .and_where(Expr::col(T::BookId).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<EntryRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.into_iter().map(EntryRow::into_entry).collect()
    }

    /// `findAllByBookId` without the bytes, in table order.
    pub async fn find_all_details_by_book(
        transaction: &mut DbTransaction,
        book_id: &BookId,
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
            .and_where(Expr::col(T::BookId).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<DetailsRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.into_iter().map(DetailsRow::into_details).collect()
    }

    /// The book and type of a thumbnail.
    pub async fn find_owner(
        transaction: &mut DbTransaction,
        id: &ThumbnailId,
    ) -> Result<Option<(BookId, ThumbnailType)>, DbError> {
        let (sql, values) = Query::select()
            .columns([T::BookId, T::Type])
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

    /// `findSelectedByBookIdOrNull`: the first selected row.
    pub async fn find_selected(
        transaction: &mut DbTransaction,
        book_id: &BookId,
    ) -> Result<Option<ThumbnailContent>, DbError> {
        let (sql, values) = Query::select()
            .columns([T::Id, T::Type, T::Thumbnail, T::Url, T::MediaType])
            .from(T::Table)
            .and_where(Expr::col(T::BookId).eq(book_id))
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
        book_id: &BookId,
        thumbnail: &NewThumbnail<'_>,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(T::Table)
            .columns([
                T::Id,
                T::BookId,
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
                book_id.into(),
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

    /// `markSelected`: selects this row and unselects the book's others.
    pub async fn mark_selected(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        id: &ThumbnailId,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(T::Table)
            .value(T::Selected, Expr::col(T::Id).eq(id))
            .and_where(Expr::col(T::BookId).eq(book_id))
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

    /// `deleteByBookIdAndType`.
    pub async fn delete_by_book_and_type(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        kind: ThumbnailType,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::delete()
            .from_table(T::Table)
            .and_where(Expr::col(T::BookId).eq(book_id))
            .and_where(Expr::col(T::Type).eq(kind.name()))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// The files of the book's thumbnails of type `kind` read from one.
    pub async fn find_urls_by_book_and_type(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        kind: ThumbnailType,
    ) -> Result<Vec<FileUrl>, DbError> {
        let (sql, values) = Query::select()
            .column(T::Url)
            .from(T::Table)
            .and_where(Expr::col(T::BookId).eq(book_id))
            .and_where(Expr::col(T::Type).eq(kind.name()))
            .and_where(Expr::col(T::Url).is_not_null())
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }

    /// `findAllBookIdsByThumbnailTypeAndDimensionSmallerThan`: both edges below `size`.
    pub async fn find_book_ids_by_type_smaller_than(
        transaction: &mut DbTransaction,
        kind: ThumbnailType,
        size: u32,
    ) -> Result<Vec<BookId>, DbError> {
        let (sql, values) = Query::select()
            .column(T::BookId)
            .from(T::Table)
            .and_where(Expr::col(T::Type).eq(kind.name()))
            .and_where(Expr::col(T::Width).lt(size))
            .and_where(Expr::col(T::Height).lt(size))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }

    /// Deletes the sidecar thumbnails read from `urls`, returning the books they belonged to.
    pub async fn delete_sidecars_by_urls(
        transaction: &mut DbTransaction,
        urls: &[&FileUrl],
    ) -> Result<Vec<BookId>, DbError> {
        let mut book_ids = Vec::new();
        for chunk in urls.chunks(DELETE_CHUNK) {
            let (sql, values) = Query::delete()
                .from_table(T::Table)
                .and_where(Expr::col(T::Type).eq(ThumbnailType::Sidecar.name()))
                .and_where(Expr::col(T::Url).is_in(chunk.iter().copied()))
                .returning_col(T::BookId)
                .build_sqlx(SqliteQueryBuilder);
            let ids: Vec<BookId> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?;
            book_ids.extend(ids);
        }
        Ok(book_ids)
    }
}
