//! Kotlin `findGeneric`: distinct values of a metadata table over visible series and books.

use crate::iden::{
    BookMetadataAuthorTable, BookTable, CollectionSeriesTable, SeriesMetadataTable, SeriesTable,
    SetKey, SetTable,
};
use komga_types::{ContentScope, FilterBy, PageRequest, PageResult, StringOperator};
use sea_query::{
    Alias, Asterisk, DynIden, Expr, ExprTrait, Func, IntoIden, Order, Query, SelectStatement,
    SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::sqlite::SqliteRow;
use sqlx::{AssertSqlSafe, FromRow};

use crate::connection::Database;
use crate::error::DbError;
use crate::support::{content_restrictions, search_operator};

/// The entity a referential table holds its values for.
#[derive(Debug, Clone, Copy)]
pub(super) enum Owner {
    Series,
    Book,
}

/// `value` contains `search`, ignoring accents and ASCII case.
pub(super) fn contains(value: Expr, search: &str) -> Expr {
    let operator = StringOperator::Contains {
        value: search.to_owned(),
    };
    search_operator::string(&operator, value)
}

/// The distinct `values` of `table`, whose rows belong to `owner`, over the series and
/// books visible in `scope` and matching `filter`.
pub(super) fn values_query(
    table: DynIden,
    owner: Owner,
    values: &[Expr],
    scope: &ContentScope,
    filter: Option<&FilterBy>,
) -> SelectStatement {
    let restrictions = content_restrictions::condition(scope.restrictions());
    let mut query = Query::select();
    query.distinct().from(table.clone());
    for value in values {
        query.expr(value.clone());
    }
    let series_needed = restrictions.is_some()
        || scope.library_ids().is_some()
        || matches!(
            filter,
            Some(FilterBy::Libraries(_) | FilterBy::Collections(_) | FilterBy::Series(_))
        );
    let (series_id, book_id) = join_owners(&mut query, &table, owner, series_needed, filter);
    if let Some(condition) = restrictions {
        if table != SeriesMetadataTable::Table.into_iden() {
            query.inner_join(
                SeriesMetadataTable::Table,
                series_id
                    .clone()
                    .equals((SeriesMetadataTable::Table, SeriesMetadataTable::SeriesId)),
            );
        }
        query.cond_where(condition);
    }
    narrow(&mut query, series_id, book_id, scope, filter);
    query
}

/// Joins BOOK when the scope or filter needs the owner `table` lacks, and returns the series and
/// book ID expressions.
fn join_owners(
    query: &mut SelectStatement,
    table: &DynIden,
    owner: Owner,
    series_needed: bool,
    filter: Option<&FilterBy>,
) -> (Expr, Expr) {
    let book_column = |column: BookTable| Expr::col((BookTable::Table, column));
    match owner {
        Owner::Series => {
            let series_id = Expr::col((table.clone(), SeriesMetadataTable::SeriesId));
            if matches!(filter, Some(FilterBy::ReadLists(_))) {
                query.inner_join(
                    BookTable::Table,
                    series_id
                        .clone()
                        .equals((BookTable::Table, BookTable::SeriesId)),
                );
            }
            (series_id, book_column(BookTable::Id))
        }
        Owner::Book => {
            let book_id = Expr::col((table.clone(), BookMetadataAuthorTable::BookId));
            if series_needed {
                query.inner_join(
                    BookTable::Table,
                    book_id.clone().equals((BookTable::Table, BookTable::Id)),
                );
            }
            (book_column(BookTable::SeriesId), book_id)
        }
    }
}

/// Keeps the rows of the scoped libraries and of the `filter` entities.
fn narrow(
    query: &mut SelectStatement,
    series_id: Expr,
    book_id: Expr,
    scope: &ContentScope,
    filter: Option<&FilterBy>,
) {
    let scoped_libraries = scope.library_ids();
    let series_library_id = || Expr::col((SeriesTable::Table, SeriesTable::LibraryId));
    if scoped_libraries.is_some_and(|ids| !ids.is_empty())
        || matches!(filter, Some(FilterBy::Libraries(_)))
    {
        query.left_join(
            SeriesTable::Table,
            series_id
                .clone()
                .equals((SeriesTable::Table, SeriesTable::Id)),
        );
    }
    // An empty set renders as an always-false condition without touching SERIES.
    if let Some(ids) = scoped_libraries {
        query.and_where(series_library_id().is_in(ids));
    }
    match filter {
        Some(FilterBy::Libraries(ids)) => {
            query.and_where(series_library_id().is_in(ids));
        }
        Some(FilterBy::Collections(ids)) => {
            let collection_id = (
                CollectionSeriesTable::Table,
                CollectionSeriesTable::CollectionId,
            );
            query
                .left_join(
                    CollectionSeriesTable::Table,
                    series_id.equals((
                        CollectionSeriesTable::Table,
                        CollectionSeriesTable::SeriesId,
                    )),
                )
                .and_where(Expr::col(collection_id).is_in(ids.iter().map(String::as_str)));
        }
        Some(FilterBy::Series(ids)) => {
            query.and_where(series_id.is_in(ids));
        }
        Some(FilterBy::ReadLists(ids)) => {
            query
                .left_join(
                    SetTable::ReadlistBook,
                    book_id.equals((SetTable::ReadlistBook, SetKey::BookId)),
                )
                .and_where(Expr::col((SetTable::ReadlistBook, SetKey::ReadlistId)).is_in(ids));
        }
        None => {}
    }
}

/// The requested page of `query`'s rows ordered by `order`, and the number of rows.
pub(super) async fn fetch_page<R>(
    database: &Database,
    query: SelectStatement,
    order: &[(Expr, Order)],
    page: PageRequest,
) -> Result<PageResult<R>, DbError>
where
    R: for<'r> FromRow<'r, SqliteRow> + Send + Unpin,
{
    let mut page_query = query.clone();
    for (expr, direction) in order {
        page_query.order_by_expr(expr.clone(), direction.clone());
    }
    if let PageRequest::Paged { size, .. } = page {
        page_query.limit(u64::from(size)).offset(page.offset());
    }
    let (sql, values) = page_query.build_sqlx(SqliteQueryBuilder);
    let items: Vec<R> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_all(database.pool())
        .await?;

    let total_elements = match page {
        PageRequest::Unpaged => items.len() as u64,
        PageRequest::Paged { .. } => {
            let (sql, values) = Query::select()
                .expr(Func::count(Expr::col(Asterisk)))
                .from_subquery(query, Alias::new("q"))
                .build_sqlx(SqliteQueryBuilder);
            let count: i64 = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_one(database.pool())
                .await?;
            u64::try_from(count).expect("COUNT(*) is never negative")
        }
    };
    Ok(PageResult {
        items,
        total_elements,
    })
}

/// Unwraps the rows of a single-column page.
pub(super) fn single_column<T>(page: PageResult<(T,)>) -> PageResult<T> {
    PageResult {
        items: page.items.into_iter().map(|(value,)| value).collect(),
        total_elements: page.total_elements,
    }
}
