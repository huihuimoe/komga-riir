//! Series lists filtered by search conditions (Kotlin `SeriesDtoDao.findAll`).

use crate::iden::BookMetadataAggregationTable as A;
use crate::iden::ReadProgressSeriesTable as RS;
use crate::iden::SeriesMetadataTable as SD;
use crate::iden::SeriesTable as S;
use komga_types::{
    ContentScope, GroupCount, PageRequest, PageResult, SeriesCondition, SeriesDetails, SeriesSort,
    SortDirection, SortOrder, UserId,
};
use sea_query::{
    Alias, Asterisk, Expr, ExprTrait, Func, Order, SelectStatement, SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::condition::SeriesFilter;
use super::{SeriesDtoDao, fetch, select_details};
use crate::sqlite::collation;
use crate::support::rows::count;
use crate::{DbError, DbTransaction, SearchCandidates};

impl SeriesDtoDao {
    /// `findAll(SeriesSearch, SearchContext, Pageable)`: the series in `scope` matching
    /// `condition`, among `candidates` when given, in the requested order and then by ID.
    ///
    /// Ordering by collection number applies only when the condition selects a collection.
    pub async fn find_all(
        transaction: &mut DbTransaction,
        user_id: &UserId,
        scope: &ContentScope,
        condition: Option<&SeriesCondition>,
        candidates: Option<&SearchCandidates>,
        sort: &[SortOrder<SeriesSort>],
        page: PageRequest,
    ) -> Result<PageResult<SeriesDetails>, DbError> {
        let filter = SeriesFilter::new(condition, scope);
        let mut query = select_details(user_id);
        filter.apply(&mut query);
        if let Some(candidates) = candidates {
            candidates.restrict(&mut query, (S::Table, S::Id));
        }
        find_page(transaction, query, sort, &filter, candidates, page).await
    }

    /// `findAllRecentlyUpdated` without full-text search: like [`find_all`](Self::find_all),
    /// without the series never modified since their creation.
    pub async fn find_recently_updated(
        transaction: &mut DbTransaction,
        user_id: &UserId,
        scope: &ContentScope,
        condition: Option<&SeriesCondition>,
        sort: &[SortOrder<SeriesSort>],
        page: PageRequest,
    ) -> Result<PageResult<SeriesDetails>, DbError> {
        let filter = SeriesFilter::new(condition, scope);
        let mut query = select_details(user_id);
        filter.apply(&mut query);
        query.and_where(
            Expr::col((S::Table, S::CreatedDate)).ne(Expr::col((S::Table, S::LastModifiedDate))),
        );
        find_page(transaction, query, sort, &filter, None, page).await
    }

    /// `countByFirstCharacter`: the series in `scope` matching `condition`, among `candidates`
    /// when given, counted by the lowercased first character of their title sort, by group.
    pub async fn count_by_first_character(
        transaction: &mut DbTransaction,
        user_id: &UserId,
        scope: &ContentScope,
        condition: Option<&SeriesCondition>,
        candidates: Option<&SearchCandidates>,
    ) -> Result<Vec<GroupCount>, DbError> {
        let filter = SeriesFilter::new(condition, scope);
        let mut query = select_details(user_id);
        filter.apply(&mut query);
        if let Some(candidates) = candidates {
            candidates.restrict(&mut query, (S::Table, S::Id));
        }
        let first_character = Func::lower(Expr::cust_with_expr(
            "substr(?, 1, 1)",
            Expr::col((SD::Table, SD::TitleSort)),
        ));
        query
            .clear_selects()
            .expr_as(first_character, Alias::new("GROUP_KEY"))
            .expr(Func::count(Expr::col(Asterisk)))
            .group_by_col(Alias::new("GROUP_KEY"))
            .order_by(Alias::new("GROUP_KEY"), Order::Asc);
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let rows: Vec<(String, i64)> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        Ok(rows
            .into_iter()
            .map(|(group, count)| GroupCount {
                group,
                count: u64::try_from(count).expect("COUNT(*) is never negative"),
            })
            .collect())
    }
}

/// The requested page of the series `query` selects, and their total.
async fn find_page(
    transaction: &mut DbTransaction,
    query: SelectStatement,
    sort: &[SortOrder<SeriesSort>],
    filter: &SeriesFilter,
    candidates: Option<&SearchCandidates>,
    page: PageRequest,
) -> Result<PageResult<SeriesDetails>, DbError> {
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
    sort: &[SortOrder<SeriesSort>],
    filter: &SeriesFilter,
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
    query.order_by((S::Table, S::Id), Order::Asc);
}

/// `SeriesDtoDao.sorts`: names and title sorts compare under the `names` collation.
fn sort_expression(
    key: SeriesSort,
    filter: &SeriesFilter,
    names: &str,
    candidates: Option<&SearchCandidates>,
) -> Option<Expr> {
    let by_name = |column| collation::collate(column, names);
    Some(match key {
        SeriesSort::TitleSort => by_name(Expr::col((SD::Table, SD::TitleSort))),
        SeriesSort::CreatedDate => Expr::col((S::Table, S::CreatedDate)),
        SeriesSort::LastModifiedDate => Expr::col((S::Table, S::LastModifiedDate)),
        SeriesSort::ReleaseDate => Expr::col((A::Table, A::ReleaseDate)),
        SeriesSort::ReadDate => Expr::col((RS::Table, RS::MostRecentReadDate)),
        SeriesSort::CollectionNumber => return filter.collection_number(),
        SeriesSort::Name => by_name(Expr::col((S::Table, S::Name))),
        SeriesSort::BooksCount => Expr::col((S::Table, S::BookCount)),
        SeriesSort::Random => Func::random().into(),
        SeriesSort::Relevance => candidates?.rank(),
    })
}
