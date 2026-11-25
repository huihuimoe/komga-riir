//! Kotlin `BookDao`.

pub(crate) mod dto;
mod metadata;
mod metadata_aggregation;
mod queries;

use crate::iden::BookTable as B;
use komga_types::{Book, BookId, FileUrl, LibraryId, SeriesId};
use sea_query::{Expr, ExprTrait, Query, SelectStatement, SimpleExpr, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::support::local_date_time::{self, SqlDateTime};
use crate::{DbError, DbTransaction};

pub use dto::{BookDtoDao, Sibling};
pub use metadata::{BookMetadataDao, BookNumbering};
pub use metadata_aggregation::BookMetadataAggregationDao;

pub struct BookDao;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookHash {
    File,
    Koreader,
}

pub(crate) const COLUMNS: [B; 12] = [
    B::Id,
    B::Name,
    B::Url,
    B::Number,
    B::FileLastModified,
    B::FileSize,
    B::FileHash,
    B::FileHashKoreader,
    B::LibraryId,
    B::SeriesId,
    B::DeletedDate,
    B::Oneshot,
];

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) struct BookRow {
    pub(crate) id: BookId,
    name: String,
    url: FileUrl,
    number: i32,
    file_last_modified: SqlDateTime,
    file_size: i64,
    file_hash: String,
    file_hash_koreader: String,
    library_id: LibraryId,
    series_id: SeriesId,
    deleted_date: Option<SqlDateTime>,
    /// Declared lowercase by Kotlin's `V20230724114349__oneshots.sql`.
    #[sqlx(rename = "oneshot")]
    oneshot: bool,
}

impl BookRow {
    pub(crate) fn into_book(self) -> Result<Book, DbError> {
        Ok(Book {
            id: self.id,
            name: self.name,
            url: self.url,
            file_last_modified: self.file_last_modified.0,
            file_size: self.file_size,
            file_hash: self.file_hash,
            file_hash_koreader: self.file_hash_koreader,
            number: self.number,
            series_id: self.series_id,
            library_id: self.library_id,
            deleted_date: self.deleted_date.map(|date| date.0),
            oneshot: self.oneshot,
        })
    }
}

impl BookDao {
    /// `findByIdOrNull`, including a soft-deleted book.
    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        id: &BookId,
    ) -> Result<Option<Book>, DbError> {
        let query = Query::select()
            .columns(COLUMNS)
            .from(B::Table)
            .and_where(Expr::col(B::Id).eq(id))
            .take();
        Ok(Self::find(transaction, query).await?.pop())
    }

    /// Books of a library, including soft-deleted ones.
    pub async fn find_all_by_library(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
    ) -> Result<Vec<Book>, DbError> {
        let query = Query::select()
            .columns(COLUMNS)
            .from(B::Table)
            .and_where(Expr::col(B::LibraryId).eq(library_id))
            .take();
        Self::find(transaction, query).await
    }

    /// `findAllBySeriesId`, including soft-deleted books.
    pub async fn find_all_by_series(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<Vec<Book>, DbError> {
        let query = Query::select()
            .columns(COLUMNS)
            .from(B::Table)
            .and_where(Expr::col(B::SeriesId).eq(series_id))
            .take();
        Self::find(transaction, query).await
    }

    /// How many of `ids` exist, including soft-deleted books.
    pub async fn count_by_ids(
        transaction: &mut DbTransaction,
        ids: &[BookId],
    ) -> Result<usize, DbError> {
        let (sql, values) = Query::select()
            .expr(Expr::col(B::Id).count())
            .from(B::Table)
            .and_where(Expr::col(B::Id).is_in(ids))
            .build_sqlx(SqliteQueryBuilder);
        let count: i64 = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?;
        Ok(usize::try_from(count).expect("COUNT is never negative"))
    }

    /// `insert`: creation and modification dates take the table defaults.
    pub async fn insert(transaction: &mut DbTransaction, books: &[Book]) -> Result<(), DbError> {
        for book in books {
            let (sql, values) = Query::insert()
                .into_table(B::Table)
                .columns(COLUMNS)
                .values_panic(values(book))
                .build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }

    pub async fn update_file_hash(
        transaction: &mut DbTransaction,
        id: &BookId,
        file_hash: &str,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(B::Table)
            .value(B::FileHash, file_hash)
            .value(B::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(B::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `update`: rewrites every column and stamps `LAST_MODIFIED_DATE`.
    pub async fn update(transaction: &mut DbTransaction, book: &Book) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(B::Table)
            .values(COLUMNS.into_iter().zip(values(book)).skip(1))
            .value(B::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(B::Id).eq(&book.id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    pub(super) async fn find(
        transaction: &mut DbTransaction,
        query: SelectStatement,
    ) -> Result<Vec<Book>, DbError> {
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let rows: Vec<BookRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.into_iter().map(BookRow::into_book).collect()
    }
}

/// Values in `COLUMNS` order.
fn values(book: &Book) -> [SimpleExpr; 12] {
    [
        (&book.id).into(),
        book.name.as_str().into(),
        (&book.url).into(),
        book.number.into(),
        SqlDateTime(book.file_last_modified).into(),
        book.file_size.into(),
        book.file_hash.as_str().into(),
        book.file_hash_koreader.as_str().into(),
        (&book.library_id).into(),
        (&book.series_id).into(),
        book.deleted_date.map(SqlDateTime).into(),
        book.oneshot.into(),
    ]
}

#[cfg(test)]
mod tests;
