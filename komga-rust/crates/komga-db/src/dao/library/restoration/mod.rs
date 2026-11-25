//! The database side of `LibraryContentLifecycle.tryRestoreBooks` and `tryRestoreSeries`: a
//! soft-deleted book or series found again as a new one hands over its state, then is deleted.
//! Moving the rows has the effect of Kotlin's copies followed by deletion.

use crate::iden::{
    BookMetadataTable as BM, BookTable as B, ContentKey as K, LibraryContentTable as T,
    ReadProgressTable as R, SeriesMetadataTable as SM, SetKey, SetTable, ThumbnailBookTable as TB,
    ThumbnailSeriesTable as TS,
};
use komga_types::{BookId, SeriesId, UserId};
use sea_query::{
    Expr, ExprTrait, IntoColumnRef, IntoTableRef, Query, SelectStatement, SimpleExpr,
    SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::deletion::{DeletedContent, delete_books, delete_series};
use crate::dao::read_progress::aggregate_series_progress;
use crate::support::local_date_time;
use crate::{DbError, DbTransaction, MediaDao};

pub struct RestoreDao;

impl RestoreDao {
    /// Restores the soft-deleted book `from` into the new book `to`. The media, generated and
    /// uploaded thumbnails, metadata, read progress, read-list places, and KOReader hash move to
    /// `to`. The metadata keeps the title of `to` unless `from` locked its own. Returns whether
    /// it kept the title of `to`.
    pub async fn restore_book(
        transaction: &mut DbTransaction,
        from: &BookId,
        to: &BookId,
    ) -> Result<bool, DbError> {
        let title = Query::select()
            .column(BM::Title)
            .from(BM::Table)
            .and_where(Expr::col(BM::BookId).eq(to))
            .take();
        let (sql, values) = Query::update()
            .table(BM::Table)
            .value(BM::Title, title)
            .and_where(Expr::col(BM::BookId).eq(from))
            .and_where(Expr::col(BM::TitleLock).eq(false))
            .build_sqlx(SqliteQueryBuilder);
        let kept_title = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?
            .rows_affected()
            == 1;
        let koreader_hash = Query::select()
            .column(B::FileHashKoreader)
            .from(B::Table)
            .and_where(Expr::col(B::Id).eq(from))
            .take();
        let book = Query::update()
            .table(B::Table)
            .value(B::FileHashKoreader, koreader_hash)
            .value(B::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(B::Id).eq(to))
            .take();
        execute(transaction, &book).await?;
        hand_over_book(transaction, from, to, &["GENERATED", "USER_UPLOADED"]).await?;
        Ok(kept_title)
    }

    /// Kotlin `BookImporter` upgrading the book `from` with the imported book `to`. The media,
    /// marked outdated for the new file, uploaded thumbnails, metadata, read progress, and
    /// read-list places move to `to`, then `from` is deleted.
    pub async fn upgrade_book(
        transaction: &mut DbTransaction,
        from: &BookId,
        to: &BookId,
    ) -> Result<(), DbError> {
        hand_over_book(transaction, from, to, &["USER_UPLOADED"]).await?;
        MediaDao::update_status(transaction, to, "OUTDATED").await
    }

    /// Restores the soft-deleted series `from` into the new series `to`, once their books are
    /// restored. The metadata, uploaded thumbnails, and collection places move to `to`, then
    /// `from` is deleted with any book left. The metadata keeps the title and sort title of `to`
    /// unless `from` locked its own. Returns what the deletion of `from` removed.
    pub async fn restore_series(
        transaction: &mut DbTransaction,
        from: &SeriesId,
        to: &SeriesId,
    ) -> Result<DeletedContent, DbError> {
        for (column, lock) in [
            (SM::Title, SM::TitleLock),
            (SM::TitleSort, SM::TitleSortLock),
        ] {
            let new_value = Query::select()
                .column(column)
                .from(SM::Table)
                .and_where(Expr::col(SM::SeriesId).eq(to))
                .take();
            let update = Query::update()
                .table(SM::Table)
                .value(column, new_value)
                .and_where(Expr::col(SM::SeriesId).eq(from))
                .and_where(Expr::col(lock).eq(false))
                .take();
            execute(transaction, &update).await?;
        }
        let collections = Query::select()
            .column(SetKey::CollectionId)
            .from(SetTable::CollectionSeries)
            .and_where(Expr::col(K::SeriesId).eq(from))
            .take();
        touch(transaction, SetTable::Collection, SetKey::Id, collections).await?;

        for table in [
            T::SeriesMetadata,
            T::SeriesMetadataGenre,
            T::SeriesMetadataTag,
            T::SeriesMetadataSharing,
            T::SeriesMetadataLink,
            T::SeriesMetadataAlternateTitle,
            T::CollectionSeries,
        ] {
            replace(transaction, table, K::SeriesId, from, to).await?;
        }
        let target = Query::select().expr(Expr::val(to)).take();
        touch(transaction, T::SeriesMetadata, K::SeriesId, target).await?;
        let thumbnails = Query::update()
            .table(TS::Table)
            .value(TS::SeriesId, to)
            .and_where(Expr::col(TS::SeriesId).eq(from))
            .and_where(Expr::col(TS::Type).eq("USER_UPLOADED"))
            .take();
        execute(transaction, &thumbnails).await?;
        delete_series(transaction, Query::select().expr(Expr::val(from)).take()).await
    }
}

/// Moves the state of book `from` to book `to`, with the thumbnails of `thumbnail_types`, then
/// deletes `from`.
async fn hand_over_book(
    transaction: &mut DbTransaction,
    from: &BookId,
    to: &BookId,
    thumbnail_types: &[&str],
) -> Result<(), DbError> {
    let (sql, values) = Query::select()
        .distinct()
        .column(R::UserId)
        .from(R::Table)
        .and_where(Expr::col(R::BookId).eq(from))
        .build_sqlx(SqliteQueryBuilder);
    let readers: Vec<UserId> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_all(transaction.connection())
        .await?;
    let read_lists = Query::select()
        .column(SetKey::ReadlistId)
        .from(SetTable::ReadlistBook)
        .and_where(Expr::col(K::BookId).eq(from))
        .take();
    touch(transaction, SetTable::Readlist, SetKey::Id, read_lists).await?;

    for table in [
        T::Media,
        T::MediaPage,
        T::MediaFile,
        T::BookMetadata,
        T::BookMetadataAuthor,
        T::BookMetadataNotableEntities,
        T::BookMetadataTag,
        T::BookMetadataLink,
        T::ReadProgress,
        T::ReadlistBook,
    ] {
        replace(transaction, table, K::BookId, from, to).await?;
    }
    for table in [T::Media, T::BookMetadata, T::ReadProgress] {
        let target = Query::select().expr(Expr::val(to)).take();
        touch(transaction, table, K::BookId, target).await?;
    }
    let thumbnails = Query::update()
        .table(TB::Table)
        .value(TB::BookId, to)
        .and_where(Expr::col(TB::BookId).eq(from))
        .and_where(Expr::col(TB::Type).is_in(thumbnail_types.iter().copied()))
        .take();
    execute(transaction, &thumbnails).await?;

    for reader in &readers {
        aggregate_series_progress(transaction, to, reader).await?;
        aggregate_series_progress(transaction, from, reader).await?;
    }
    delete_books(transaction, Query::select().expr(Expr::val(from)).take()).await?;
    Ok(())
}

/// Replaces the rows of `to` in `table` with those of `from`.
async fn replace(
    transaction: &mut DbTransaction,
    table: T,
    key: K,
    from: impl Into<SimpleExpr> + Copy,
    to: impl Into<SimpleExpr> + Copy,
) -> Result<(), DbError> {
    let delete = Query::delete()
        .from_table(table)
        .and_where(Expr::col(key).eq(to))
        .take();
    execute(transaction, &delete).await?;
    let update = Query::update()
        .table(table)
        .value(key, to)
        .and_where(Expr::col(key).eq(from))
        .take();
    execute(transaction, &update).await
}

/// Stamps `LAST_MODIFIED_DATE` on the rows of `table` whose `key` is in `keys`.
async fn touch(
    transaction: &mut DbTransaction,
    table: impl IntoTableRef,
    key: impl IntoColumnRef,
    keys: SelectStatement,
) -> Result<(), DbError> {
    let update = Query::update()
        .table(table)
        .value(K::LastModifiedDate, local_date_time::now_utc())
        .and_where(Expr::col(key).in_subquery(keys))
        .take();
    execute(transaction, &update).await
}

async fn execute(
    transaction: &mut DbTransaction,
    statement: &impl SqlxBinder,
) -> Result<(), DbError> {
    let (sql, values) = statement.build_sqlx(SqliteQueryBuilder);
    sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
        .execute(transaction.connection())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
