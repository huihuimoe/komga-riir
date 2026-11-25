//! Book lists filtered by search conditions (Kotlin `BookDtoDao.findAll`).

use crate::iden::BookMetadataTable as D;
use crate::iden::BookTable as B;
use crate::iden::MediaTable as M;
use crate::iden::ReadProgressTable as R;
use crate::iden::SeriesMetadataTable as SD;
use komga_types::{
    BookCondition, BookDetails, BookId, BookSort, ContentScope, PageRequest, PageResult,
    SortDirection, SortOrder, UserId,
};
use sea_query::{Expr, ExprTrait, Order, Query, SelectStatement, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::condition::BookFilter;
use super::{BookDtoDao, Sibling, count, fetch, select_details};
use crate::sqlite::collation;
use crate::{DbError, DbTransaction, SearchCandidates};

impl BookDtoDao {
    /// `findAll(BookSearch, SearchContext, Pageable)`: the books in `scope` matching
    /// `condition`, among `candidates` when given, in the requested order and then by ID.
    ///
    /// Ordering by read list number applies only when the condition selects a read list.
    pub async fn find_all(
        transaction: &mut DbTransaction,
        user_id: &UserId,
        scope: &ContentScope,
        condition: Option<&BookCondition>,
        candidates: Option<&SearchCandidates>,
        sort: &[SortOrder<BookSort>],
        page: PageRequest,
    ) -> Result<PageResult<BookDetails>, DbError> {
        let filter = BookFilter::new(condition, scope);
        let mut query = select_details(user_id);
        filter.apply(&mut query);
        if let Some(candidates) = candidates {
            candidates.restrict(&mut query, (B::Table, B::Id));
        }
        find_page(transaction, query, sort, &filter, candidates, page).await
    }

    /// `findAllDuplicates`: the books in `scope` sharing their file hash and size with another
    /// book, in the requested order and then by ID.
    pub async fn find_duplicates(
        transaction: &mut DbTransaction,
        user_id: &UserId,
        scope: &ContentScope,
        sort: &[SortOrder<BookSort>],
        page: PageRequest,
    ) -> Result<PageResult<BookDetails>, DbError> {
        let filter = BookFilter::new(None, scope);
        let mut query = select_details(user_id);
        filter.apply(&mut query);
        let duplicated = Query::select()
            .columns([B::FileHash, B::FileSize])
            .from(B::Table)
            .and_where(Expr::col(B::FileHash).ne(""))
            .group_by_columns([B::FileHash, B::FileSize])
            .and_having(Expr::col(B::Id).count().gt(1))
            .take();
        query.and_where(
            Expr::tuple([
                Expr::col((B::Table, B::FileHash)),
                Expr::col((B::Table, B::FileSize)),
            ])
            .in_subquery(duplicated),
        );
        find_page(transaction, query, sort, &filter, None, page).await
    }

    /// The book before or after `book_id` among the books [`find_all`](Self::find_all) lists for
    /// the same arguments; `None` when `book_id` is not among them or has no such neighbour.
    pub async fn find_sibling(
        transaction: &mut DbTransaction,
        user_id: &UserId,
        scope: &ContentScope,
        condition: &BookCondition,
        sort: &[SortOrder<BookSort>],
        book_id: &BookId,
        sibling: Sibling,
    ) -> Result<Option<BookDetails>, DbError> {
        let filter = BookFilter::new(Some(condition), scope);
        let mut query = select_details(user_id);
        filter.apply(&mut query);
        query.clear_selects();
        query.column((B::Table, B::Id));
        let names = transaction.sort_rules().collation(filter.library());
        order(&mut query, sort, &filter, &names, None);
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let ids: Vec<BookId> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        let Some(index) = ids.iter().position(|id| id == book_id) else {
            return Ok(None);
        };
        let neighbour = match sibling {
            Sibling::Previous => index.checked_sub(1).and_then(|index| ids.get(index)),
            Sibling::Next => ids.get(index + 1),
        };
        match neighbour {
            Some(id) => Self::find_by_id(transaction, id, user_id).await,
            None => Ok(None),
        }
    }
}

/// The requested page of the books `query` selects, and their total.
async fn find_page(
    transaction: &mut DbTransaction,
    query: SelectStatement,
    sort: &[SortOrder<BookSort>],
    filter: &BookFilter,
    candidates: Option<&SearchCandidates>,
    page: PageRequest,
) -> Result<PageResult<BookDetails>, DbError> {
    let names = transaction.sort_rules().collation(filter.library());
    let mut page_query = query.clone();
    order(&mut page_query, sort, filter, &names, candidates);
    if let PageRequest::Paged { size, .. } = page {
        page_query.limit(u64::from(size)).offset(page.offset());
    }
    let items = fetch(transaction, &page_query).await?;
    let total_elements = match page {
        PageRequest::Paged { .. } => count(transaction, query).await?,
        PageRequest::Unpaged => items.len() as u64,
    };
    Ok(PageResult {
        items,
        total_elements,
    })
}

/// Orders `query` as requested, then by ID.
fn order(
    query: &mut SelectStatement,
    sort: &[SortOrder<BookSort>],
    filter: &BookFilter,
    names: &str,
    candidates: Option<&SearchCandidates>,
) {
    for order in sort {
        if let Some(expression) = sort_expression(order.key, filter, names, candidates) {
            let direction = match order.direction {
                SortDirection::Asc => Order::Asc,
                SortDirection::Desc => Order::Desc,
            };
            query.order_by_expr(expression, direction);
        }
    }
    query.order_by((B::Table, B::Id), Order::Asc);
}

/// `BookDtoDao.sorts`: names and titles sort under the `names` collation, other text ignoring
/// ASCII case.
fn sort_expression(
    key: BookSort,
    filter: &BookFilter,
    names: &str,
    candidates: Option<&SearchCandidates>,
) -> Option<Expr> {
    let by_name = |column| collation::collate(column, names);
    let no_case = |column| collation::collate(column, "NOCASE");
    Some(match key {
        BookSort::Name => by_name(Expr::col((B::Table, B::Name))),
        BookSort::SeriesTitleSort => by_name(Expr::col((SD::Table, SD::TitleSort))),
        BookSort::CreatedDate => Expr::col((B::Table, B::CreatedDate)),
        BookSort::LastModifiedDate => Expr::col((B::Table, B::LastModifiedDate)),
        BookSort::FileSize => Expr::col((B::Table, B::FileSize)),
        BookSort::FileHash => Expr::col((B::Table, B::FileHash)),
        BookSort::Url => no_case(Expr::col((B::Table, B::Url))),
        BookSort::MediaStatus => no_case(Expr::col((M::Table, M::Status))),
        BookSort::MediaComment => no_case(Expr::col((M::Table, M::Comment))),
        BookSort::MediaType => no_case(Expr::col((M::Table, M::MediaType))),
        BookSort::MediaPageCount => Expr::col((M::Table, M::PageCount)),
        BookSort::Title => by_name(Expr::col((D::Table, D::Title))),
        BookSort::NumberSort => Expr::col((D::Table, D::NumberSort)),
        BookSort::ReleaseDate => Expr::col((D::Table, D::ReleaseDate)),
        BookSort::ReadProgressLastModified => Expr::col((R::Table, R::LastModifiedDate)),
        BookSort::ReadDate => Expr::col((R::Table, R::ReadDate)),
        BookSort::ReadListNumber => return filter.read_list_number(),
        BookSort::Relevance => candidates?.rank(),
    })
}

/// The books in `scope` matching `condition`, with their details.
pub(crate) fn select_matching(
    user_id: &UserId,
    scope: &ContentScope,
    condition: &BookCondition,
) -> SelectStatement {
    let mut query = select_details(user_id);
    BookFilter::new(Some(condition), scope).apply(&mut query);
    query
}
