//! Read lists as a user sees them: only books in their libraries and allowed content count.

use std::collections::{HashMap, HashSet};

use crate::iden::BookTable as B;
use crate::iden::SeriesMetadataTable as SD;
use crate::iden::{SetKey as K, SetTable as T};
use komga_types::{
    BookId, ContentScope, FileUrl, LibraryId, PageRequest, PageResult, ReadList, ReadListId,
    ReadListSort, SortDirection, SortOrder,
};
use sea_query::{
    Alias, Asterisk, Expr, ExprTrait, Func, Order, Query, SelectStatement, SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::ReadListDao;
use crate::sqlite::collation;
use crate::support::content_restrictions;
use crate::support::local_date_time::SqlDateTime;
use crate::support::rows::BATCH_CHUNK_SIZE;
use crate::{DbError, DbTransaction, SearchCandidates};

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct ReadListRow {
    id: ReadListId,
    name: String,
    summary: String,
    ordered: bool,
    book_count: i32,
    created_date: SqlDateTime,
    last_modified_date: SqlDateTime,
}

impl ReadListDao {
    /// The books among `book_ids` that `scope` allows, in the same order.
    pub async fn visible_books(
        transaction: &mut DbTransaction,
        book_ids: &[BookId],
        scope: &ContentScope,
    ) -> Result<Vec<BookId>, DbError> {
        if !is_scoped(scope) || book_ids.is_empty() {
            return Ok(book_ids.to_vec());
        }
        let mut query = Query::select();
        query
            .column((B::Table, B::Id))
            .from(B::Table)
            .and_where(Expr::col((B::Table, B::Id)).is_in(book_ids));
        keep_books_in(&mut query, scope);
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let visible: HashSet<BookId> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?
            .into_iter()
            .collect();
        Ok(book_ids
            .iter()
            .filter(|id| visible.contains(*id))
            .cloned()
            .collect())
    }

    /// `findByIdOrNull`: within a user's `scope`, only read lists holding a book they may see,
    /// with only those books; without, every read list and book.
    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        id: &ReadListId,
        scope: Option<&ContentScope>,
    ) -> Result<Option<ReadList>, DbError> {
        let scope = scope.filter(|scope| is_scoped(scope));
        let mut query = Query::select();
        query.from(T::Readlist);
        if scope.is_some() {
            join_books(&mut query, scope);
        }
        query
            .distinct()
            .columns(COLUMNS.map(|column| (T::Readlist, column)))
            .and_where(Expr::col((T::Readlist, K::Id)).eq(id));
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let row: Option<ReadListRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        match row {
            Some(row) => Ok(read_lists(transaction, vec![row], scope).await?.pop()),
            None => Ok(None),
        }
    }

    /// `findAll(SearchContext(user), pageable, belongsToLibraryIds, search)`: the read lists
    /// holding a book `scope` allows, in `libraries` and among `candidates` when given, each with
    /// the books `scope` allows. The ID breaks ties of the requested order.
    pub async fn find_all(
        transaction: &mut DbTransaction,
        scope: &ContentScope,
        libraries: Option<&[LibraryId]>,
        candidates: Option<&SearchCandidates>,
        sort: &[SortOrder<ReadListSort>],
        page: PageRequest,
    ) -> Result<PageResult<ReadList>, DbError> {
        let library = match libraries {
            Some([library]) => Some(library.as_str()),
            _ => None,
        };
        let names = transaction
            .sort_rules()
            .collation(collation::sole_library(scope, library));
        let scope = Some(scope).filter(|scope| is_scoped(scope));
        let mut query = Query::select();
        query.from(T::Readlist);
        if let Some(candidates) = candidates {
            candidates.restrict(&mut query, (T::Readlist, K::Id));
        }
        if scope.is_some() || libraries.is_some() {
            join_books(&mut query, scope);
        }
        if let Some(libraries) = libraries {
            query.and_where(Expr::col((B::Table, B::LibraryId)).is_in(libraries));
        }
        let mut page_query = query.clone();
        page_query
            .distinct()
            .columns(COLUMNS.map(|column| (T::Readlist, column)));
        for order in sort {
            let direction = match order.direction {
                SortDirection::Asc => Order::Asc,
                SortDirection::Desc => Order::Desc,
            };
            if let Some(expression) = sort_expression(order.key, &names, candidates) {
                page_query.order_by_expr(expression, direction);
            }
        }
        page_query.order_by((T::Readlist, K::Id), Order::Asc);
        if let PageRequest::Paged { size, .. } = page {
            page_query.limit(u64::from(size)).offset(page.offset());
        }
        let (sql, values) = page_query.build_sqlx(SqliteQueryBuilder);
        let rows: Vec<ReadListRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        let items = read_lists(transaction, rows, scope).await?;
        let total_elements = match page {
            PageRequest::Unpaged => items.len() as u64,
            PageRequest::Paged { .. } => {
                query.distinct().column((T::Readlist, K::Id));
                let (sql, values) = Query::select()
                    .expr(Func::count(Expr::col(Asterisk)))
                    .from_subquery(query, Alias::new("q"))
                    .build_sqlx(SqliteQueryBuilder);
                let count: i64 = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                    .fetch_one(transaction.connection())
                    .await?;
                u64::try_from(count).expect("COUNT(*) is never negative")
            }
        };
        Ok(PageResult {
            items,
            total_elements,
        })
    }

    /// `findAllContainingBookId`: the read lists holding book `book_id`, as
    /// [`find_by_id`](Self::find_by_id) shows them within `scope`, by ID.
    pub async fn find_all_containing_book(
        transaction: &mut DbTransaction,
        book_id: &BookId,
        scope: &ContentScope,
    ) -> Result<Vec<ReadList>, DbError> {
        let scope = Some(scope).filter(|scope| is_scoped(scope));
        let mut query = Query::select();
        query.from(T::Readlist);
        if scope.is_some() {
            join_books(&mut query, scope);
        }
        query
            .distinct()
            .columns(COLUMNS.map(|column| (T::Readlist, column)))
            .and_where(
                Expr::col((T::Readlist, K::Id)).in_subquery(
                    Query::select()
                        .column(K::ReadlistId)
                        .from(T::ReadlistBook)
                        .and_where(Expr::col(K::BookId).eq(book_id))
                        .take(),
                ),
            )
            .order_by((T::Readlist, K::Id), Order::Asc);
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let rows: Vec<ReadListRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        read_lists(transaction, rows, scope).await
    }

    /// `(NUMBER, URL)` of the books of read list `id` that `scope` allows, by number.
    pub async fn find_book_urls(
        transaction: &mut DbTransaction,
        id: &ReadListId,
        scope: &ContentScope,
    ) -> Result<Vec<(i32, FileUrl)>, DbError> {
        let mut query = Query::select();
        query
            .column((T::ReadlistBook, K::Number))
            .column((B::Table, B::Url))
            .from(T::ReadlistBook)
            .and_where(Expr::col((T::ReadlistBook, K::ReadlistId)).eq(id))
            .order_by((T::ReadlistBook, K::Number), Order::Asc);
        restrict_books(&mut query, Some(scope).filter(|scope| is_scoped(scope)));
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }
}

const COLUMNS: [K; 7] = [
    K::Id,
    K::Name,
    K::Summary,
    K::Ordered,
    K::BookCount,
    K::CreatedDate,
    K::LastModifiedDate,
];

/// `ReadListDao.sorts`; names compare under the `names` collation.
fn sort_expression(
    key: ReadListSort,
    names: &str,
    candidates: Option<&SearchCandidates>,
) -> Option<Expr> {
    Some(match key {
        ReadListSort::Name => Expr::cust(format!(r#""READLIST"."NAME" COLLATE {names}"#)),
        ReadListSort::CreatedDate => Expr::col((T::Readlist, K::CreatedDate)),
        ReadListSort::LastModifiedDate => Expr::col((T::Readlist, K::LastModifiedDate)),
        ReadListSort::Relevance => candidates?.rank(),
    })
}

async fn read_lists(
    transaction: &mut DbTransaction,
    rows: Vec<ReadListRow>,
    scope: Option<&ContentScope>,
) -> Result<Vec<ReadList>, DbError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<ReadListId> = rows.iter().map(|row| row.id.clone()).collect();
    let mut books_by_list = visible_books_by_ids(transaction, &ids, scope).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let book_ids = books_by_list.remove(&row.id).unwrap_or_default();
        items.push(ReadList {
            filtered: usize::try_from(row.book_count).ok() != Some(book_ids.len()),
            id: row.id,
            name: row.name,
            summary: row.summary,
            ordered: row.ordered,
            book_ids,
            created_date: row.created_date.0,
            last_modified_date: row.last_modified_date.0,
        });
    }
    Ok(items)
}

/// The books of each read list in `ids` by number, restricted to `scope` if it limits anything.
async fn visible_books_by_ids(
    transaction: &mut DbTransaction,
    ids: &[ReadListId],
    scope: Option<&ContentScope>,
) -> Result<HashMap<ReadListId, Vec<BookId>>, DbError> {
    let mut grouped: HashMap<ReadListId, Vec<BookId>> = HashMap::with_capacity(ids.len());
    for chunk in ids.chunks(BATCH_CHUNK_SIZE) {
        let mut query = Query::select();
        query
            .columns([
                (T::ReadlistBook, K::ReadlistId),
                (T::ReadlistBook, K::BookId),
            ])
            .from(T::ReadlistBook)
            .and_where(Expr::col((T::ReadlistBook, K::ReadlistId)).is_in(chunk))
            .order_by((T::ReadlistBook, K::ReadlistId), Order::Asc)
            .order_by((T::ReadlistBook, K::Number), Order::Asc);
        if scope.is_some() {
            restrict_books(&mut query, scope);
        }
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let pairs: Vec<(ReadListId, BookId)> =
            sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?;
        for (list_id, book_id) in pairs {
            grouped.entry(list_id).or_default().push(book_id);
        }
    }
    Ok(grouped)
}

fn is_scoped(scope: &ContentScope) -> bool {
    scope.library_ids().is_some() || scope.restrictions().is_restricted()
}

/// Joins the books of the read lists in `query` and keeps the rows of books in `scope`, if any.
fn join_books(query: &mut SelectStatement, scope: Option<&ContentScope>) {
    query.left_join(
        T::ReadlistBook,
        Expr::col((T::Readlist, K::Id)).equals((T::ReadlistBook, K::ReadlistId)),
    );
    restrict_books(query, scope);
}

/// Joins the books of the `READLIST_BOOK` rows in `query` and keeps those in `scope`, if any.
fn restrict_books(query: &mut SelectStatement, scope: Option<&ContentScope>) {
    query.left_join(
        B::Table,
        Expr::col((T::ReadlistBook, K::BookId)).equals((B::Table, B::Id)),
    );
    if let Some(scope) = scope {
        keep_books_in(query, scope);
    }
}

/// Keeps the rows of the books in `query` that `scope` allows.
fn keep_books_in(query: &mut SelectStatement, scope: &ContentScope) {
    if let Some(condition) = content_restrictions::condition(scope.restrictions()) {
        query
            .left_join(
                SD::Table,
                Expr::col((SD::Table, SD::SeriesId)).equals((B::Table, B::SeriesId)),
            )
            .cond_where(condition);
    }
    if let Some(ids) = scope.library_ids() {
        query.and_where(Expr::col((B::Table, B::LibraryId)).is_in(ids));
    }
}
