//! Kotlin `ReadProgressDao`.

use crate::iden::BookMetadataTable as D;
use crate::iden::BookTable as B;
use crate::iden::MediaTable as M;
use crate::iden::ReadProgressSeriesTable as RS;
use crate::iden::ReadProgressTable as R;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use komga_types::{BookId, ReadProgress, SeriesId, UserId};
use sea_query::{
    Cond, Condition, Expr, ExprTrait, Func, OnConflict, Query, SelectStatement, SqliteQueryBuilder,
    Value,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;
use tracing::warn;

mod dto;

use crate::support::json_gz;
use crate::support::local_date_time::{self, SqlDateTime};
use crate::{DbError, DbTransaction};

pub use dto::{NumberedCompletion, ReadProgressDtoDao};

pub struct ReadProgressDao;

/// The columns a saved progress inserts; all but the key are replaced on conflict.
const PROGRESS_COLUMNS: [R; 8] = [
    R::BookId,
    R::UserId,
    R::Page,
    R::Completed,
    R::ReadDate,
    R::DeviceId,
    R::DeviceName,
    R::Locator,
];

fn replace_progress() -> OnConflict {
    OnConflict::columns([R::BookId, R::UserId])
        .update_columns(PROGRESS_COLUMNS.into_iter().skip(2))
        .value(R::LastModifiedDate, local_date_time::now_utc())
        .to_owned()
}

/// `(PAGE, COMPLETED, READ_DATE, DEVICE_ID, DEVICE_NAME, LOCATOR)`.
type ProgressRow = (
    i32,
    bool,
    Option<SqlDateTime>,
    String,
    String,
    Option<Vec<u8>>,
);

impl ReadProgressDao {
    /// Kotlin `findByBookIdAndUserIdOrNull`; a locator that does not decode is dropped, as Kotlin
    /// does.
    pub async fn find(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        user_id: &UserId,
    ) -> Result<Option<ReadProgress>, DbError> {
        let (sql, values) = Query::select()
            .columns([
                R::Page,
                R::Completed,
                R::ReadDate,
                R::DeviceId,
                R::DeviceName,
                R::Locator,
            ])
            .from(R::Table)
            .and_where(Expr::col(R::BookId).eq(book_id))
            .and_where(Expr::col(R::UserId).eq(user_id))
            .build_sqlx(SqliteQueryBuilder);
        let row: Option<ProgressRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        let Some((page, completed, read_date, device_id, device_name, locator)) = row else {
            return Ok(None);
        };
        let read_date = read_date.ok_or_else(|| {
            DbError::UnsupportedSchema(format!("Read progress of book {book_id} has no read date"))
        })?;
        let read_date = read_date
            .0
            .to_zoned_datetime(TimeZone::UTC)
            .map_err(|error| DbError::UnsupportedSchema(format!("Invalid read date: {error}")))?
            .timestamp();
        let locator = locator.and_then(|blob| {
            json_gz::deserialize(&blob)
                .inspect_err(|error| warn!(book = %book_id, %error, "Cannot read locator"))
                .ok()
        });
        Ok(Some(ReadProgress {
            book_id: book_id.clone(),
            user_id: user_id.clone(),
            page,
            completed,
            read_date,
            device_id,
            device_name,
            locator,
        }))
    }

    /// Kotlin `save(readProgress)`: inserts or replaces the user's progress on the book, then
    /// aggregates the user's series progress again.
    pub async fn save(
        transaction: &mut DbTransaction,
        progress: &ReadProgress,
    ) -> Result<(), DbError> {
        let read_date = SqlDateTime(
            progress
                .read_date
                .to_zoned_datetime(TimeZone::UTC)
                .datetime(),
        );
        let locator = progress.locator.as_ref().map(json_gz::serialize);
        let (sql, values) = Query::insert()
            .into_table(R::Table)
            .columns(PROGRESS_COLUMNS)
            .values_panic([
                (&progress.book_id).into(),
                (&progress.user_id).into(),
                progress.page.into(),
                progress.completed.into(),
                read_date.into(),
                progress.device_id.as_str().into(),
                progress.device_name.as_str().into(),
                locator.into(),
            ])
            .on_conflict(replace_progress())
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        aggregate_series_progress(transaction, &progress.book_id, &progress.user_id).await
    }

    /// Kotlin `delete(bookId, userId)`: removes the user's progress on the book, then aggregates
    /// the user's series progress again. Returns whether there was one.
    pub async fn delete(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        user_id: &UserId,
    ) -> Result<bool, DbError> {
        let (sql, values) = Query::delete()
            .from_table(R::Table)
            .and_where(Expr::col(R::BookId).eq(book_id))
            .and_where(Expr::col(R::UserId).eq(user_id))
            .build_sqlx(SqliteQueryBuilder);
        let deleted = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?
            .rows_affected();
        aggregate_series_progress(transaction, book_id, user_id).await?;
        Ok(deleted > 0)
    }

    /// What `BookLifecycle.analyzeAndPersist` saves when a re-analysis changes the page count:
    /// every progress on the book moves to the new last page when completed and to page 1
    /// otherwise, then the series progress of each affected user is aggregated again.
    pub async fn reset_pages(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        page_count: i32,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::select()
            .distinct()
            .column(R::UserId)
            .from(R::Table)
            .and_where(Expr::col(R::BookId).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        let user_ids: Vec<UserId> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;

        let (sql, values) = Query::update()
            .table(R::Table)
            .value(
                R::Page,
                Expr::case(Expr::col(R::Completed).eq(true), page_count).finally(1),
            )
            .value(R::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(R::BookId).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;

        for user_id in &user_ids {
            aggregate_series_progress(transaction, book_id, user_id).await?;
        }
        Ok(())
    }

    /// Kotlin `SeriesLifecycle.markReadProgressCompleted`: every book of the series, soft-deleted
    /// ones included, that the user has not completed is saved as completed on its last page at
    /// `read_date`; completed books keep their progress. Books without media are skipped. With
    /// `up_to_number_sort`, only books whose `NUMBER_SORT` is not above it, as the Mihon update
    /// does. Returns the books completed.
    pub async fn complete_series(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
        up_to_number_sort: Option<f32>,
        user_id: &UserId,
        read_date: Timestamp,
    ) -> Result<Vec<BookId>, DbError> {
        let mut books = Cond::all().add(Expr::col((B::Table, B::SeriesId)).eq(series_id));
        if let Some(number_sort) = up_to_number_sort {
            let numbered = Query::select()
                .column(D::BookId)
                .from(D::Table)
                .and_where(Expr::col(D::NumberSort).lte(f64::from(number_sort)))
                .take();
            books = books.add(Expr::col((B::Table, B::Id)).in_subquery(numbered));
        }
        complete(transaction, books, user_id, read_date).await
    }

    /// `markReadProgressCompleted` of each of the books the user has not completed, as
    /// [`Self::complete_series`] does. Returns the books completed.
    pub async fn complete_books(
        transaction: &mut DbTransaction,
        book_ids: &[BookId],
        user_id: &UserId,
        read_date: Timestamp,
    ) -> Result<Vec<BookId>, DbError> {
        let books = Cond::all().add(Expr::col((B::Table, B::Id)).is_in(book_ids));
        complete(transaction, books, user_id, read_date).await
    }

    /// Kotlin `SeriesLifecycle.deleteReadProgress`: removes the user's progress on every book of
    /// the series, soft-deleted ones included. Returns the books that had one.
    pub async fn delete_series(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
        user_id: &UserId,
    ) -> Result<Vec<BookId>, DbError> {
        let book_ids = Query::select()
            .column(B::Id)
            .from(B::Table)
            .and_where(Expr::col(B::SeriesId).eq(series_id))
            .take();
        let (sql, values) = Query::delete()
            .from_table(R::Table)
            .and_where(Expr::col(R::UserId).eq(user_id))
            .and_where(Expr::col(R::BookId).in_subquery(book_ids))
            .returning_col(R::BookId)
            .build_sqlx(SqliteQueryBuilder);
        let deleted = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        aggregate_series(transaction, series_id, user_id).await?;
        Ok(deleted)
    }
}

/// Saves the books matching `books` (a condition on `BOOK`) that the user has not completed as
/// completed on their last page at `read_date`, then aggregates their series again. Books without
/// media are skipped. Returns the books completed.
async fn complete(
    transaction: &mut DbTransaction,
    books: Condition,
    user_id: &UserId,
    read_date: Timestamp,
) -> Result<Vec<BookId>, DbError> {
    let read_date = SqlDateTime(read_date.to_zoned_datetime(TimeZone::UTC).datetime());
    let uncompleted = Query::select()
        .column((B::Table, B::Id))
        .expr(Expr::val(user_id))
        .column((M::Table, M::PageCount))
        .expr(Expr::val(true))
        .expr(Expr::val(read_date))
        .expr(Expr::val(""))
        .expr(Expr::val(""))
        .expr(Expr::val(Value::Bytes(None)))
        .from(B::Table)
        .inner_join(
            M::Table,
            Expr::col((M::Table, M::BookId)).equals((B::Table, B::Id)),
        )
        .left_join(
            R::Table,
            Expr::col((R::Table, R::BookId))
                .equals((B::Table, B::Id))
                .and(Expr::col((R::Table, R::UserId)).eq(user_id)),
        )
        .cond_where(books.clone())
        .and_where(
            Expr::col((R::Table, R::Completed))
                .is_null()
                .or(Expr::col((R::Table, R::Completed)).eq(false)),
        )
        .take();
    // The select has a WHERE clause, so SQLite does not read ON CONFLICT as a join constraint.
    let (sql, values) = Query::insert()
        .into_table(R::Table)
        .columns(PROGRESS_COLUMNS)
        .select_from(uncompleted)
        .expect("The select has one expression per column")
        .on_conflict(replace_progress())
        .returning_col(R::BookId)
        .build_sqlx(SqliteQueryBuilder);
    let completed = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_all(transaction.connection())
        .await?;

    let series_ids = Query::select()
        .distinct()
        .column(B::SeriesId)
        .from(B::Table)
        .cond_where(books)
        .take();
    aggregate(transaction, &series_ids, user_id).await?;
    Ok(completed)
}

async fn aggregate_series(
    transaction: &mut DbTransaction,
    series_id: &SeriesId,
    user_id: &UserId,
) -> Result<(), DbError> {
    let series_ids = Query::select().expr(Expr::val(series_id)).take();
    aggregate(transaction, &series_ids, user_id).await
}

/// Kotlin `aggregateSeriesProgress` for one user: rebuilds the user's `READ_PROGRESS_SERIES` row
/// of the book's series from the book progresses of that series.
pub(crate) async fn aggregate_series_progress(
    transaction: &mut DbTransaction,
    book_id: &BookId,
    user_id: &UserId,
) -> Result<(), DbError> {
    let series_ids = Query::select()
        .column(B::SeriesId)
        .from(B::Table)
        .and_where(Expr::col(B::Id).eq(book_id))
        .take();
    aggregate(transaction, &series_ids, user_id).await
}

/// Rebuilds the user's `READ_PROGRESS_SERIES` rows of the selected series.
async fn aggregate(
    transaction: &mut DbTransaction,
    series_ids: &SelectStatement,
    user_id: &UserId,
) -> Result<(), DbError> {
    let series_ids = || series_ids.clone();

    let (sql, values) = Query::delete()
        .from_table(RS::Table)
        .and_where(Expr::col(RS::SeriesId).in_subquery(series_ids()))
        .and_where(Expr::col(RS::UserId).eq(user_id))
        .build_sqlx(SqliteQueryBuilder);
    sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
        .execute(transaction.connection())
        .await?;

    let count_when = |completed: bool| {
        Func::sum(Expr::case(Expr::col((R::Table, R::Completed)).eq(completed), 1).finally(0))
    };
    let progress = Query::select()
        .column((B::Table, B::SeriesId))
        .column((R::Table, R::UserId))
        .expr(count_when(true))
        .expr(count_when(false))
        .expr(Func::max(Expr::col((R::Table, R::ReadDate))))
        .expr(Expr::current_timestamp())
        .from(B::Table)
        .inner_join(
            R::Table,
            Expr::col((B::Table, B::Id)).equals((R::Table, R::BookId)),
        )
        .and_where(Expr::col((B::Table, B::SeriesId)).in_subquery(series_ids()))
        .and_where(Expr::col((R::Table, R::UserId)).eq(user_id))
        .group_by_col((B::Table, B::SeriesId))
        .group_by_col((R::Table, R::UserId))
        .take();
    let (sql, values) = Query::insert()
        .into_table(RS::Table)
        .columns([
            RS::SeriesId,
            RS::UserId,
            RS::ReadCount,
            RS::InProgressCount,
            RS::MostRecentReadDate,
            RS::LastModifiedDate,
        ])
        .select_from(progress)
        .expect("The select has one expression per column")
        .build_sqlx(SqliteQueryBuilder);
    sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
        .execute(transaction.connection())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
