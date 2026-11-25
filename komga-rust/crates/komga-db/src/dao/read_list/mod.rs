//! Kotlin `ReadListDao`.

use crate::iden::{SetKey as K, SetTable as T};
use komga_types::{BookId, ReadListId};
use sea_query::{Expr, ExprTrait, Func, Order, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::support::local_date_time;
use crate::{DbError, DbTransaction};

mod request;
mod visible;

pub use request::ReadListRequestDao;

pub struct ReadListDao;

/// The fields of a read list besides its books.
#[derive(Debug, Clone, Copy)]
pub struct ReadListContent<'a> {
    pub name: &'a str,
    pub summary: &'a str,
    pub ordered: bool,
}

/// A read list with every book, ordered by number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadListBooks {
    pub id: ReadListId,
    /// `(NUMBER, BOOK_ID)` pairs.
    pub books: Vec<(i32, BookId)>,
}

impl ReadListDao {
    /// `findByNameOrNull`: the read list whose name equals `name` under SQLite's ASCII-only
    /// `lower`, as jOOQ's `equalIgnoreCase` compares.
    pub async fn find_by_name(
        transaction: &mut DbTransaction,
        name: &str,
    ) -> Result<Option<ReadListBooks>, DbError> {
        let (sql, values) = Query::select()
            .column(K::Id)
            .from(T::Readlist)
            .and_where(Expr::expr(Func::lower(Expr::col(K::Name))).eq(Func::lower(name)))
            .order_by(K::Id, Order::Asc)
            .limit(1)
            .build_sqlx(SqliteQueryBuilder);
        let found: Option<ReadListId> =
            sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_optional(transaction.connection())
                .await?;
        let Some(id) = found else {
            return Ok(None);
        };
        let (sql, values) = Query::select()
            .columns([K::Number, K::BookId])
            .from(T::ReadlistBook)
            .and_where(Expr::col(K::ReadlistId).eq(&id))
            .order_by(K::Number, Order::Asc)
            .build_sqlx(SqliteQueryBuilder);
        let books: Vec<(i32, BookId)> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        Ok(Some(ReadListBooks { id, books }))
    }

    /// `insert`: a new read list holding `book` at `number`, with the default summary and
    /// manual ordering.
    pub async fn insert_with_book(
        transaction: &mut DbTransaction,
        id: &ReadListId,
        name: &str,
        number: i32,
        book: &BookId,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(T::Readlist)
            .columns([K::Id, K::Name, K::BookCount])
            .values_panic([id.into(), name.into(), 1.into()])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        insert_book(transaction, id, number, book).await
    }

    /// `update` with one more book: adds `book` at `number`, then recounts the books and stamps
    /// `LAST_MODIFIED_DATE`.
    pub async fn add_book(
        transaction: &mut DbTransaction,
        id: &ReadListId,
        number: i32,
        book: &BookId,
    ) -> Result<(), DbError> {
        insert_book(transaction, id, number, book).await?;
        let count = Query::select()
            .expr(Expr::col(K::BookId).count())
            .from(T::ReadlistBook)
            .and_where(Expr::col(K::ReadlistId).eq(id))
            .take();
        let (sql, values) = Query::update()
            .table(T::Readlist)
            .value(K::BookCount, count)
            .value(K::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(K::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `findAllEmpty` and `delete`: removes read lists without books, with their thumbnails, and
    /// returns their IDs.
    pub async fn delete_empty(transaction: &mut DbTransaction) -> Result<Vec<ReadListId>, DbError> {
        let members = Query::select()
            .column(K::ReadlistId)
            .from(T::ReadlistBook)
            .take();
        let (sql, values) = Query::select()
            .column(K::Id)
            .from(T::Readlist)
            .and_where(Expr::col(K::Id).not_in_subquery(members))
            .build_sqlx(SqliteQueryBuilder);
        let ids: Vec<ReadListId> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        delete_all(transaction, &ids).await?;
        Ok(ids)
    }

    /// `existsByName` under the same case folding as [`Self::find_by_name`], ignoring the read
    /// list `except` so that it may keep or recase its own name.
    pub async fn exists_by_name(
        transaction: &mut DbTransaction,
        name: &str,
        except: Option<&ReadListId>,
    ) -> Result<bool, DbError> {
        let mut query = Query::select();
        query
            .expr(Expr::val(1))
            .from(T::Readlist)
            .and_where(Expr::expr(Func::lower(Expr::col(K::Name))).eq(Func::lower(name)));
        if let Some(except) = except {
            query.and_where(Expr::col(K::Id).ne(except));
        }
        let (sql, values) = query.limit(1).build_sqlx(SqliteQueryBuilder);
        let found: Option<i32> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        Ok(found.is_some())
    }

    /// `insert`: the books are numbered from 0 in the given order.
    pub async fn insert(
        transaction: &mut DbTransaction,
        id: &ReadListId,
        content: &ReadListContent<'_>,
        books: &[BookId],
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(T::Readlist)
            .columns([K::Id, K::Name, K::Summary, K::Ordered, K::BookCount])
            .values_panic([
                id.into(),
                content.name.into(),
                content.summary.into(),
                content.ordered.into(),
                book_count(books).into(),
            ])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        insert_books(transaction, id, books).await
    }

    /// `update`, stamping `LAST_MODIFIED_DATE`. With `books`, they replace every member,
    /// numbered from 0; without, the members stay as they are.
    pub async fn update(
        transaction: &mut DbTransaction,
        id: &ReadListId,
        content: &ReadListContent<'_>,
        books: Option<&[BookId]>,
    ) -> Result<(), DbError> {
        let mut query = Query::update();
        query
            .table(T::Readlist)
            .value(K::Name, content.name)
            .value(K::Summary, content.summary)
            .value(K::Ordered, content.ordered)
            .value(K::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(K::Id).eq(id));
        if let Some(books) = books {
            query.value(K::BookCount, book_count(books));
        }
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        let Some(books) = books else {
            return Ok(());
        };
        let (sql, values) = Query::delete()
            .from_table(T::ReadlistBook)
            .and_where(Expr::col(K::ReadlistId).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        insert_books(transaction, id, books).await
    }

    /// `(ID, NAME)` of each of `ids` that exists, for its search document.
    pub async fn find_names(
        transaction: &mut DbTransaction,
        ids: &[&str],
    ) -> Result<Vec<(String, String)>, DbError> {
        let (sql, values) = Query::select()
            .columns([K::Id, K::Name])
            .from(T::Readlist)
            .and_where(Expr::col(K::Id).is_in(ids.iter().copied()))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }

    /// `ThumbnailReadListDao.deleteByReadListId` and `delete`.
    pub async fn delete(transaction: &mut DbTransaction, id: &ReadListId) -> Result<(), DbError> {
        delete_all(transaction, std::slice::from_ref(id)).await
    }
}

/// Deletes read lists with their members and thumbnails.
async fn delete_all(transaction: &mut DbTransaction, ids: &[ReadListId]) -> Result<(), DbError> {
    for (table, column) in [
        (T::ThumbnailReadlist, K::ReadlistId),
        (T::ReadlistBook, K::ReadlistId),
        (T::Readlist, K::Id),
    ] {
        let (sql, values) = Query::delete()
            .from_table(table)
            .and_where(Expr::col(column).is_in(ids))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
    }
    Ok(())
}

async fn insert_books(
    transaction: &mut DbTransaction,
    id: &ReadListId,
    books: &[BookId],
) -> Result<(), DbError> {
    for (number, book) in (0..).zip(books) {
        insert_book(transaction, id, number, book).await?;
    }
    Ok(())
}

fn book_count(books: &[BookId]) -> i32 {
    i32::try_from(books.len()).expect("A read list holds fewer than 2^31 books")
}

async fn insert_book(
    transaction: &mut DbTransaction,
    id: &ReadListId,
    number: i32,
    book: &BookId,
) -> Result<(), DbError> {
    let (sql, values) = Query::insert()
        .into_table(T::ReadlistBook)
        .columns([K::ReadlistId, K::BookId, K::Number])
        .values_panic([id.into(), book.into(), number.into()])
        .build_sqlx(SqliteQueryBuilder);
    sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
        .execute(transaction.connection())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
