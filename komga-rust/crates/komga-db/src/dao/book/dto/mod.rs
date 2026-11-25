//! Kotlin `BookDtoDao`: books as one user sees them.

mod condition;
mod search;

pub(crate) use search::select_matching;

use crate::iden::BookMetadataTable as D;
use crate::iden::BookTable as B;
use crate::iden::MediaTable as M;
use crate::iden::ReadProgressSeriesTable as RS;
use crate::iden::ReadProgressTable as R;
use crate::iden::SeriesMetadataTable as SD;
use crate::iden::SeriesTable as S;
use komga_types::{
    BookDetails, BookId, ContentScope, LibraryId, MediaDetails, MediaStatus, PageRequest,
    PageResult, ReadProgressDetails, SeriesId, UserId,
};
use sea_query::{
    Alias, Expr, ExprTrait, JoinType, Order, Query, SelectStatement, SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::{BookMetadataDao, BookRow, COLUMNS};
use crate::support::content_restrictions;
use crate::support::local_date_time::SqlDateTime;
use crate::support::rows::count;
use crate::{DbError, DbTransaction};

pub struct BookDtoDao;

/// Which neighbour of a book to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sibling {
    Previous,
    Next,
}

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct DetailsRow {
    #[sqlx(flatten)]
    book: BookRow,
    series_title: String,
    book_created_date: SqlDateTime,
    book_last_modified_date: SqlDateTime,
    status: MediaStatus,
    media_type: Option<String>,
    page_count: i32,
    comment: Option<String>,
    epub_divina_compatible: bool,
    epub_is_kepub: bool,
    metadata_created_date: SqlDateTime,
    metadata_last_modified_date: SqlDateTime,
    progress_page: Option<i32>,
    progress_completed: Option<bool>,
    progress_read_date: Option<SqlDateTime>,
    progress_created_date: Option<SqlDateTime>,
    progress_last_modified_date: Option<SqlDateTime>,
    progress_device_id: Option<String>,
    progress_device_name: Option<String>,
}

impl BookDtoDao {
    /// `findByIdOrNull(bookId, userId)`.
    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        user_id: &UserId,
    ) -> Result<Option<BookDetails>, DbError> {
        let mut query = select_details(user_id);
        query.and_where(Expr::col((B::Table, B::Id)).eq(book_id));
        Ok(fetch(transaction, &query).await?.pop())
    }

    /// `findPreviousInSeriesOrNull`/`findNextInSeriesOrNull`: the book of the same series with the
    /// closest lower or higher `numberSort`, the ID breaking ties so that books of equal number are
    /// not skipped; `None` for an unknown book.
    pub async fn find_sibling_in_series(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        user_id: &UserId,
        sibling: Sibling,
    ) -> Result<Option<BookDetails>, DbError> {
        let (sql, values) = Query::select()
            .column((B::Table, B::SeriesId))
            .column((D::Table, D::NumberSort))
            .from(B::Table)
            .inner_join(
                D::Table,
                Expr::col((D::Table, D::BookId)).equals((B::Table, B::Id)),
            )
            .and_where(Expr::col((B::Table, B::Id)).eq(book_id))
            .build_sqlx(SqliteQueryBuilder);
        let Some((series_id, number_sort)): Option<(SeriesId, f64)> =
            sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_optional(transaction.connection())
                .await?
        else {
            return Ok(None);
        };

        let number = || Expr::col((D::Table, D::NumberSort));
        let id = || Expr::col((B::Table, B::Id));
        let (condition, order) = match sibling {
            Sibling::Previous => (
                number()
                    .lt(number_sort)
                    .or(number().eq(number_sort).and(id().lt(book_id))),
                Order::Desc,
            ),
            Sibling::Next => (
                number()
                    .gt(number_sort)
                    .or(number().eq(number_sort).and(id().gt(book_id))),
                Order::Asc,
            ),
        };
        let mut query = select_details(user_id);
        query
            .and_where(Expr::col((B::Table, B::SeriesId)).eq(series_id))
            .and_where(condition)
            .order_by((D::Table, D::NumberSort), order.clone())
            .order_by((B::Table, B::Id), order)
            .limit(1);
        Ok(fetch(transaction, &query).await?.pop())
    }

    /// `findAllOnDeck`: of each series in `scope` (and `libraries` when given) the user has read
    /// some books of, started none, and not finished, the first book by `numberSort` they have
    /// not started; most recently read series first.
    pub async fn find_on_deck(
        transaction: &mut DbTransaction,
        user_id: &UserId,
        scope: &ContentScope,
        libraries: Option<&[LibraryId]>,
        page: PageRequest,
    ) -> Result<PageResult<BookDetails>, DbError> {
        let query = select_on_deck(user_id, scope, libraries);
        let mut page_query = query.clone();
        page_query
            .order_by((RS::Table, RS::MostRecentReadDate), Order::Desc)
            .order_by((B::Table, B::Id), Order::Asc);
        if let PageRequest::Paged { size, .. } = page {
            page_query.limit(u64::from(size)).offset(page.offset());
        }
        let items = fetch(transaction, &page_query).await?;
        Ok(PageResult {
            items,
            total_elements: count(transaction, query).await?,
        })
    }
}

/// The books `find_on_deck` returns, unordered, joined with the user's series progress.
pub(crate) fn select_on_deck(
    user_id: &UserId,
    scope: &ContentScope,
    libraries: Option<&[LibraryId]>,
) -> SelectStatement {
    let mut query = select_details(user_id);
    query
        .inner_join(
            S::Table,
            Expr::col((S::Table, S::Id)).equals((B::Table, B::SeriesId)),
        )
        .inner_join(
            RS::Table,
            Expr::col((RS::Table, RS::SeriesId))
                .equals((S::Table, S::Id))
                .and(Expr::col((RS::Table, RS::UserId)).eq(user_id)),
        )
        .and_where(Expr::col((RS::Table, RS::InProgressCount)).eq(0))
        .and_where(Expr::col((RS::Table, RS::ReadCount)).ne(Expr::col((S::Table, S::BookCount))))
        .and_where(Expr::col((B::Table, B::Id)).in_subquery(first_unread_book(user_id)));
    if let Some(condition) = content_restrictions::condition(scope.restrictions()) {
        query.cond_where(condition);
    }
    if let Some(ids) = scope.library_ids() {
        query.and_where(Expr::col((S::Table, S::LibraryId)).is_in(ids));
    }
    if let Some(ids) = libraries {
        query.and_where(Expr::col((S::Table, S::LibraryId)).is_in(ids));
    }
    query
}

/// The book of the outer query's series the user has not started, lowest `numberSort` first and
/// the ID breaking ties.
fn first_unread_book(user_id: &UserId) -> SelectStatement {
    let book = Alias::new("FIRST_BOOK");
    let metadata = Alias::new("FIRST_METADATA");
    let progress = Alias::new("FIRST_PROGRESS");
    Query::select()
        .column((book.clone(), B::Id))
        .from_as(B::Table, book.clone())
        .join_as(
            JoinType::InnerJoin,
            D::Table,
            metadata.clone(),
            Expr::col((metadata.clone(), D::BookId)).equals((book.clone(), B::Id)),
        )
        .join_as(
            JoinType::LeftJoin,
            R::Table,
            progress.clone(),
            Expr::col((progress.clone(), R::BookId))
                .equals((book.clone(), B::Id))
                .and(Expr::col((progress.clone(), R::UserId)).eq(user_id)),
        )
        .and_where(Expr::col((book.clone(), B::SeriesId)).equals((B::Table, B::SeriesId)))
        .and_where(Expr::col((progress, R::BookId)).is_null())
        .order_by((metadata, D::NumberSort), Order::Asc)
        .order_by((book, B::Id), Order::Asc)
        .limit(1)
        .take()
}

/// The book, its media, metadata, series title, and the user's progress.
fn select_details(user_id: &UserId) -> SelectStatement {
    let mut query = Query::select();
    query
        .columns(COLUMNS.map(|column| (B::Table, column)))
        .expr_as(
            Expr::col((SD::Table, SD::Title)),
            Alias::new("SERIES_TITLE"),
        )
        .expr_as(
            Expr::col((B::Table, B::CreatedDate)),
            Alias::new("BOOK_CREATED_DATE"),
        )
        .expr_as(
            Expr::col((B::Table, B::LastModifiedDate)),
            Alias::new("BOOK_LAST_MODIFIED_DATE"),
        )
        .columns([
            (M::Table, M::Status),
            (M::Table, M::MediaType),
            (M::Table, M::PageCount),
            (M::Table, M::Comment),
            (M::Table, M::EpubDivinaCompatible),
            (M::Table, M::EpubIsKepub),
        ])
        .expr_as(
            Expr::col((D::Table, D::CreatedDate)),
            Alias::new("METADATA_CREATED_DATE"),
        )
        .expr_as(
            Expr::col((D::Table, D::LastModifiedDate)),
            Alias::new("METADATA_LAST_MODIFIED_DATE"),
        );
    for (column, name) in [
        (R::Page, "PROGRESS_PAGE"),
        (R::Completed, "PROGRESS_COMPLETED"),
        (R::ReadDate, "PROGRESS_READ_DATE"),
        (R::CreatedDate, "PROGRESS_CREATED_DATE"),
        (R::LastModifiedDate, "PROGRESS_LAST_MODIFIED_DATE"),
        (R::DeviceId, "PROGRESS_DEVICE_ID"),
        (R::DeviceName, "PROGRESS_DEVICE_NAME"),
    ] {
        query.expr_as(Expr::col((R::Table, column)), Alias::new(name));
    }
    query
        .from(B::Table)
        .inner_join(
            M::Table,
            Expr::col((M::Table, M::BookId)).equals((B::Table, B::Id)),
        )
        .inner_join(
            D::Table,
            Expr::col((D::Table, D::BookId)).equals((B::Table, B::Id)),
        )
        .inner_join(
            SD::Table,
            Expr::col((SD::Table, SD::SeriesId)).equals((B::Table, B::SeriesId)),
        )
        .left_join(
            R::Table,
            Expr::col((R::Table, R::BookId))
                .equals((B::Table, B::Id))
                .and(Expr::col((R::Table, R::UserId)).eq(user_id)),
        );
    query
}

async fn fetch(
    transaction: &mut DbTransaction,
    query: &SelectStatement,
) -> Result<Vec<BookDetails>, DbError> {
    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
    let rows: Vec<DetailsRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_all(transaction.connection())
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let book_ids: Vec<BookId> = rows.iter().map(|row| row.book.id.clone()).collect();
    let mut metadata_by_book = BookMetadataDao::find_by_ids(transaction, &book_ids).await?;
    let mut books = Vec::with_capacity(rows.len());
    for row in rows {
        let read_progress = read_progress(&row)?;
        let book = row.book.into_book()?;
        let metadata = metadata_by_book
            .remove(&book.id)
            .expect("The query joins the metadata of the book");
        books.push(BookDetails {
            series_title: row.series_title,
            created: row.book_created_date.0,
            last_modified: row.book_last_modified_date.0,
            media: MediaDetails {
                status: row.status,
                media_type: row.media_type,
                page_count: row.page_count,
                comment: row.comment,
                epub_divina_compatible: row.epub_divina_compatible,
                epub_is_kepub: row.epub_is_kepub,
            },
            metadata,
            metadata_created: row.metadata_created_date.0,
            metadata_last_modified: row.metadata_last_modified_date.0,
            read_progress,
            book,
        });
    }
    Ok(books)
}

/// `None` without a progress row; Kotlin requires a read date on one.
fn read_progress(row: &DetailsRow) -> Result<Option<ReadProgressDetails>, DbError> {
    let Some(page) = row.progress_page else {
        return Ok(None);
    };
    let missing = |column: &str| {
        DbError::UnsupportedSchema(format!(
            "Read progress of book {} has no {column}",
            row.book.id
        ))
    };
    let date = |value: Option<SqlDateTime>, column: &str| {
        value.map(|date| date.0).ok_or_else(|| missing(column))
    };
    Ok(Some(ReadProgressDetails {
        page,
        completed: row
            .progress_completed
            .ok_or_else(|| missing("completion"))?,
        read_date: date(row.progress_read_date, "read date")?,
        created: date(row.progress_created_date, "creation date")?,
        last_modified: date(row.progress_last_modified_date, "modification date")?,
        device_id: row
            .progress_device_id
            .clone()
            .ok_or_else(|| missing("device ID"))?,
        device_name: row
            .progress_device_name
            .clone()
            .ok_or_else(|| missing("device name"))?,
    }))
}
