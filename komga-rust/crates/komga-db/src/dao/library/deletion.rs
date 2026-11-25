//! Library content cleanup: `LibraryLifecycle.deleteLibrary` and
//! `LibraryContentLifecycle.emptyTrash` through `SeriesLifecycle.deleteMany` and
//! `BookLifecycle.deleteMany`.

use crate::iden::{
    ContentKey as K, LibraryContentTable as T, LibraryExclusionsTable, LibraryTable,
    UserLibrarySharingTable,
};
use komga_types::{BookRef, LibraryId, SearchEntity, SeriesId, SeriesRef};
use sea_query::{Expr, ExprTrait, Query, SelectStatement, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::LibraryDao;
use crate::{DbError, DbTransaction, SearchIndexPendingDao};

impl LibraryDao {
    /// Removes database content only; source files, historical events, and sync snapshots remain.
    /// Returns the series and books deleted with the library.
    pub async fn delete(
        transaction: &mut DbTransaction,
        id: &LibraryId,
    ) -> Result<DeletedContent, DbError> {
        let series = Query::select()
            .column(K::Id)
            .from(T::Series)
            .and_where(Expr::col(K::LibraryId).eq(id))
            .take();
        let deleted = delete_series(transaction, series).await?;
        let queries = [
            Query::delete()
                .from_table(T::Sidecar)
                .and_where(Expr::col(K::LibraryId).eq(id))
                .take(),
            Query::delete()
                .from_table(LibraryExclusionsTable::Table)
                .and_where(Expr::col(LibraryExclusionsTable::LibraryId).eq(id))
                .take(),
            Query::delete()
                .from_table(UserLibrarySharingTable::Table)
                .and_where(Expr::col(UserLibrarySharingTable::LibraryId).eq(id))
                .take(),
            Query::delete()
                .from_table(LibraryTable::Table)
                .and_where(Expr::col(LibraryTable::Id).eq(id))
                .take(),
        ];
        for query in queries {
            let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(deleted)
    }

    /// `LibraryContentLifecycle.emptyTrash` database cleanup: deletes the library's soft-deleted
    /// series with all their books, then its remaining soft-deleted books. Returns what it
    /// deleted, and the surviving series that lost books, which need their books renumbered.
    pub async fn delete_trash(
        transaction: &mut DbTransaction,
        id: &LibraryId,
    ) -> Result<(DeletedContent, Vec<SeriesId>), DbError> {
        let series = Query::select()
            .column(K::Id)
            .from(T::Series)
            .and_where(Expr::col(K::LibraryId).eq(id))
            .and_where(Expr::col(K::DeletedDate).is_not_null())
            .take();
        let mut deleted = delete_series(transaction, series).await?;
        let books = Query::select()
            .column(K::Id)
            .from(T::Book)
            .and_where(Expr::col(K::LibraryId).eq(id))
            .and_where(Expr::col(K::DeletedDate).is_not_null())
            .take();
        let (sql, values) = Query::select()
            .distinct()
            .column(K::SeriesId)
            .from(T::Book)
            .and_where(Expr::col(K::Id).in_subquery(books.clone()))
            .build_sqlx(SqliteQueryBuilder);
        let affected_series: Vec<SeriesId> =
            sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?;
        deleted
            .books
            .extend(delete_books(transaction, books).await?);
        Ok((deleted, affected_series))
    }
}

/// Series and books a deletion removed, books first, for the events of the caller.
#[derive(Debug, Default)]
pub struct DeletedContent {
    pub books: Vec<BookRef>,
    pub series: Vec<SeriesRef>,
}

/// `SeriesLifecycle.deleteMany`: the series, their books, and series-owned rows. Their search
/// documents are marked out of date.
pub(super) async fn delete_series(
    transaction: &mut DbTransaction,
    series: SelectStatement,
) -> Result<DeletedContent, DbError> {
    let (sql, values) = Query::select()
        .columns([K::Id, K::LibraryId])
        .from(T::Series)
        .and_where(Expr::col(K::Id).in_subquery(series.clone()))
        .build_sqlx(SqliteQueryBuilder);
    let deleted_series: Vec<(SeriesId, LibraryId)> =
        sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
    let condition = Expr::col(K::Id).in_subquery(series.clone());
    SearchIndexPendingDao::mark_rows(transaction, SearchEntity::Series, T::Series, condition)
        .await?;
    let books = Query::select()
        .column(K::Id)
        .from(T::Book)
        .and_where(Expr::col(K::SeriesId).in_subquery(series.clone()))
        .take();
    let books = delete_books(transaction, books).await?;
    for table in [
        T::ReadProgressSeries,
        T::CollectionSeries,
        T::ThumbnailSeries,
        T::SeriesMetadataGenre,
        T::SeriesMetadataTag,
        T::SeriesMetadataSharing,
        T::SeriesMetadataLink,
        T::SeriesMetadataAlternateTitle,
        T::SeriesMetadata,
        T::BookMetadataAggregationAuthor,
        T::BookMetadataAggregationNotableEntities,
        T::BookMetadataAggregationTag,
        T::BookMetadataAggregation,
    ] {
        delete_children(transaction, table, K::SeriesId, series.clone()).await?;
    }
    delete_children(transaction, T::Series, K::Id, series).await?;
    Ok(DeletedContent {
        books,
        series: deleted_series
            .into_iter()
            .map(|(series_id, library_id)| SeriesRef {
                series_id,
                library_id,
            })
            .collect(),
    })
}

/// `BookLifecycle.deleteMany`: the books and book-owned rows, returning them. Their search
/// documents are marked out of date.
pub(super) async fn delete_books(
    transaction: &mut DbTransaction,
    books: SelectStatement,
) -> Result<Vec<BookRef>, DbError> {
    let (sql, values) = Query::select()
        .columns([K::Id, K::SeriesId, K::LibraryId])
        .from(T::Book)
        .and_where(Expr::col(K::Id).in_subquery(books.clone()))
        .build_sqlx(SqliteQueryBuilder);
    let deleted: Vec<(_, _, _)> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_all(transaction.connection())
        .await?;
    let condition = Expr::col(K::Id).in_subquery(books.clone());
    SearchIndexPendingDao::mark_rows(transaction, SearchEntity::Book, T::Book, condition).await?;
    for table in [
        T::ReadProgress,
        T::ReadlistBook,
        T::MediaPage,
        T::MediaFile,
        T::Media,
        T::ThumbnailBook,
        T::BookMetadataAuthor,
        T::BookMetadataNotableEntities,
        T::BookMetadataTag,
        T::BookMetadataLink,
        T::BookMetadata,
        T::BookProjection,
    ] {
        delete_children(transaction, table, K::BookId, books.clone()).await?;
    }
    delete_children(transaction, T::Book, K::Id, books).await?;
    Ok(deleted
        .into_iter()
        .map(|(book_id, series_id, library_id)| BookRef {
            book_id,
            series_id,
            library_id,
        })
        .collect())
}

async fn delete_children(
    transaction: &mut DbTransaction,
    table: T,
    key: K,
    children: SelectStatement,
) -> Result<(), DbError> {
    let (sql, values) = Query::delete()
        .from_table(table)
        .and_where(Expr::col(key).in_subquery(children))
        .build_sqlx(SqliteQueryBuilder);
    sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
        .execute(transaction.connection())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::LibraryDao;
    use crate::Database;

    #[tokio::test]
    async fn delete_trash_removes_only_the_librarys_soft_deleted_content() {
        let directory = komga_test_support::tempdir().expect("Database directory");
        let db = Database::connect(&directory.path().join("database.sqlite"))
            .await
            .expect("Open database");
        db.migrate().await.expect("Migrate database");
        let mut tx = db.begin_immediate().await.expect("Begin");
        for statement in [
            "INSERT INTO LIBRARY (ID, NAME, ROOT) VALUES ('l', 'L', 'file:/l'), ('o', 'O', 'file:/o')",
            "INSERT INTO SERIES (ID, FILE_LAST_MODIFIED, NAME, URL, LIBRARY_ID, DELETED_DATE) VALUES \
             ('trashed', CURRENT_TIMESTAMP, 'T', 'file:/l/t', 'l', CURRENT_TIMESTAMP), \
             ('alive', CURRENT_TIMESTAMP, 'A', 'file:/l/a', 'l', NULL), \
             ('other', CURRENT_TIMESTAMP, 'O', 'file:/o/o', 'o', CURRENT_TIMESTAMP)",
            "INSERT INTO BOOK (ID, FILE_LAST_MODIFIED, NAME, URL, SERIES_ID, LIBRARY_ID, DELETED_DATE) \
             VALUES \
             ('in-trashed', CURRENT_TIMESTAMP, '1', 'file:/l/t/1', 'trashed', 'l', NULL), \
             ('trashed-book', CURRENT_TIMESTAMP, '2', 'file:/l/a/2', 'alive', 'l', CURRENT_TIMESTAMP), \
             ('alive-book', CURRENT_TIMESTAMP, '3', 'file:/l/a/3', 'alive', 'l', NULL), \
             ('other-book', CURRENT_TIMESTAMP, '4', 'file:/o/o/4', 'other', 'o', CURRENT_TIMESTAMP)",
        ] {
            sqlx::query(statement)
                .execute(tx.connection())
                .await
                .expect("Fixture");
        }
        let renumbered = LibraryDao::delete_trash(&mut tx, &"l".into())
            .await
            .expect("Delete trash");
        assert_eq!(renumbered.1, [komga_types::SeriesId::from("alive")]);
        let series: Vec<String> = sqlx::query_scalar("SELECT ID FROM SERIES ORDER BY ID")
            .fetch_all(tx.connection())
            .await
            .expect("Series");
        assert_eq!(series, ["alive", "other"]);
        let books: Vec<String> = sqlx::query_scalar("SELECT ID FROM BOOK ORDER BY ID")
            .fetch_all(tx.connection())
            .await
            .expect("Books");
        assert_eq!(books, ["alive-book", "other-book"]);
    }
}
