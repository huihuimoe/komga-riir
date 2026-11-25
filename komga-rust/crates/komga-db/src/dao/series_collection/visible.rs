//! Collections as a user sees them: only series in their libraries and allowed content count.

use std::collections::{HashMap, HashSet};

use crate::iden::SeriesMetadataTable as SD;
use crate::iden::SeriesTable as S;
use crate::iden::{SetKey as K, SetTable as T};
use komga_types::{
    CollectionId, CollectionSort, ContentScope, LibraryId, PageRequest, PageResult,
    SeriesCollection, SeriesId, SortDirection, SortOrder,
};
use sea_query::{
    Alias, Asterisk, Expr, ExprTrait, Func, Order, Query, SelectStatement, SqliteQueryBuilder,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::SeriesCollectionDao;
use crate::sqlite::collation;
use crate::support::content_restrictions;
use crate::support::local_date_time::SqlDateTime;
use crate::support::rows::BATCH_CHUNK_SIZE;
use crate::{DbError, DbTransaction, SearchCandidates};

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
struct CollectionRow {
    id: CollectionId,
    name: String,
    ordered: bool,
    series_count: i32,
    created_date: SqlDateTime,
    last_modified_date: SqlDateTime,
}

const COLUMNS: [K; 6] = [
    K::Id,
    K::Name,
    K::Ordered,
    K::SeriesCount,
    K::CreatedDate,
    K::LastModifiedDate,
];

impl SeriesCollectionDao {
    /// The series among `series_ids` that `scope` allows, in the same order.
    pub async fn visible_series(
        transaction: &mut DbTransaction,
        series_ids: &[SeriesId],
        scope: &ContentScope,
    ) -> Result<Vec<SeriesId>, DbError> {
        if !is_scoped(scope) || series_ids.is_empty() {
            return Ok(series_ids.to_vec());
        }
        let mut query = Query::select();
        query
            .column((S::Table, S::Id))
            .from(S::Table)
            .and_where(Expr::col((S::Table, S::Id)).is_in(series_ids));
        keep_series_in(&mut query, scope);
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let visible: HashSet<SeriesId> =
            sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?
                .into_iter()
                .collect();
        Ok(series_ids
            .iter()
            .filter(|id| visible.contains(*id))
            .cloned()
            .collect())
    }

    /// `findByIdOrNull`: within a user's `scope`, only collections holding a series they may
    /// see, with only those series; without, every collection and series.
    pub async fn find_by_id(
        transaction: &mut DbTransaction,
        id: &CollectionId,
        scope: Option<&ContentScope>,
    ) -> Result<Option<SeriesCollection>, DbError> {
        let scope = scope.filter(|scope| is_scoped(scope));
        let mut query = Query::select();
        query.from(T::Collection);
        if scope.is_some() {
            join_series(&mut query, scope);
        }
        query
            .distinct()
            .columns(COLUMNS.map(|column| (T::Collection, column)))
            .and_where(Expr::col((T::Collection, K::Id)).eq(id));
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let row: Option<CollectionRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        match row {
            Some(row) => Ok(collections(transaction, vec![row], scope).await?.pop()),
            None => Ok(None),
        }
    }

    /// `findAll(SearchContext(user), pageable, belongsToLibraryIds, search)`: the collections
    /// holding a series `scope` allows, in `libraries` and among `candidates` when given, each
    /// with the series `scope` allows. The ID breaks ties of the requested order.
    pub async fn find_all(
        transaction: &mut DbTransaction,
        scope: &ContentScope,
        libraries: Option<&[LibraryId]>,
        candidates: Option<&SearchCandidates>,
        sort: &[SortOrder<CollectionSort>],
        page: PageRequest,
    ) -> Result<PageResult<SeriesCollection>, DbError> {
        let library = match libraries {
            Some([library]) => Some(library.as_str()),
            _ => None,
        };
        let names = transaction
            .sort_rules()
            .collation(collation::sole_library(scope, library));
        let scope = Some(scope).filter(|scope| is_scoped(scope));
        let mut query = Query::select();
        query.from(T::Collection);
        if let Some(candidates) = candidates {
            candidates.restrict(&mut query, (T::Collection, K::Id));
        }
        if scope.is_some() || libraries.is_some() {
            join_series(&mut query, scope);
        }
        if let Some(libraries) = libraries {
            query.and_where(Expr::col((S::Table, S::LibraryId)).is_in(libraries));
        }
        let mut page_query = query.clone();
        page_query
            .distinct()
            .columns(COLUMNS.map(|column| (T::Collection, column)));
        for order in sort {
            let direction = match order.direction {
                SortDirection::Asc => Order::Asc,
                SortDirection::Desc => Order::Desc,
            };
            if let Some(expression) = sort_expression(order.key, &names, candidates) {
                page_query.order_by_expr(expression, direction);
            }
        }
        page_query.order_by((T::Collection, K::Id), Order::Asc);
        if let PageRequest::Paged { size, .. } = page {
            page_query.limit(u64::from(size)).offset(page.offset());
        }
        let (sql, values) = page_query.build_sqlx(SqliteQueryBuilder);
        let rows: Vec<CollectionRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        let items = collections(transaction, rows, scope).await?;
        let total_elements = match page {
            PageRequest::Unpaged => items.len() as u64,
            PageRequest::Paged { .. } => {
                query.distinct().column((T::Collection, K::Id));
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

    /// `findAllContainingSeriesId`: the collections holding series `series_id`, as
    /// [`find_by_id`](Self::find_by_id) shows them within `scope`, by ID.
    pub async fn find_all_containing_series(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
        scope: &ContentScope,
    ) -> Result<Vec<SeriesCollection>, DbError> {
        let scope = Some(scope).filter(|scope| is_scoped(scope));
        let mut query = Query::select();
        query.from(T::Collection);
        if scope.is_some() {
            join_series(&mut query, scope);
        }
        query
            .distinct()
            .columns(COLUMNS.map(|column| (T::Collection, column)))
            .and_where(
                Expr::col((T::Collection, K::Id)).in_subquery(
                    Query::select()
                        .column(K::CollectionId)
                        .from(T::CollectionSeries)
                        .and_where(Expr::col(K::SeriesId).eq(series_id))
                        .take(),
                ),
            )
            .order_by((T::Collection, K::Id), Order::Asc);
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let rows: Vec<CollectionRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        collections(transaction, rows, scope).await
    }
}

/// `SeriesCollectionDao.sorts`; names compare under the `names` collation.
fn sort_expression(
    key: CollectionSort,
    names: &str,
    candidates: Option<&SearchCandidates>,
) -> Option<Expr> {
    Some(match key {
        CollectionSort::Name => Expr::cust(format!(r#""COLLECTION"."NAME" COLLATE {names}"#)),
        CollectionSort::CreatedDate => Expr::col((T::Collection, K::CreatedDate)),
        CollectionSort::LastModifiedDate => Expr::col((T::Collection, K::LastModifiedDate)),
        CollectionSort::Relevance => candidates?.rank(),
    })
}

async fn collections(
    transaction: &mut DbTransaction,
    rows: Vec<CollectionRow>,
    scope: Option<&ContentScope>,
) -> Result<Vec<SeriesCollection>, DbError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<CollectionId> = rows.iter().map(|row| row.id.clone()).collect();
    let mut series_by_collection = visible_series_by_ids(transaction, &ids, scope).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let series_ids = series_by_collection.remove(&row.id).unwrap_or_default();
        items.push(SeriesCollection {
            filtered: usize::try_from(row.series_count).ok() != Some(series_ids.len()),
            id: row.id,
            name: row.name,
            ordered: row.ordered,
            series_ids,
            created_date: row.created_date.0,
            last_modified_date: row.last_modified_date.0,
        });
    }
    Ok(items)
}

/// The series of each collection in `ids` by number, restricted to `scope` if it limits anything.
async fn visible_series_by_ids(
    transaction: &mut DbTransaction,
    ids: &[CollectionId],
    scope: Option<&ContentScope>,
) -> Result<HashMap<CollectionId, Vec<SeriesId>>, DbError> {
    let mut grouped: HashMap<CollectionId, Vec<SeriesId>> = HashMap::with_capacity(ids.len());
    for chunk in ids.chunks(BATCH_CHUNK_SIZE) {
        let mut query = Query::select();
        query
            .columns([
                (T::CollectionSeries, K::CollectionId),
                (T::CollectionSeries, K::SeriesId),
            ])
            .from(T::CollectionSeries)
            .and_where(Expr::col((T::CollectionSeries, K::CollectionId)).is_in(chunk))
            .order_by((T::CollectionSeries, K::CollectionId), Order::Asc)
            .order_by((T::CollectionSeries, K::Number), Order::Asc);
        if scope.is_some() {
            restrict_series(&mut query, scope);
        }
        let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
        let pairs: Vec<(CollectionId, SeriesId)> =
            sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?;
        for (collection_id, series_id) in pairs {
            grouped.entry(collection_id).or_default().push(series_id);
        }
    }
    Ok(grouped)
}

fn is_scoped(scope: &ContentScope) -> bool {
    scope.library_ids().is_some() || scope.restrictions().is_restricted()
}

/// Joins the series of the collections in `query` and keeps the rows of series in `scope`, if
/// any.
fn join_series(query: &mut SelectStatement, scope: Option<&ContentScope>) {
    query.left_join(
        T::CollectionSeries,
        Expr::col((T::Collection, K::Id)).equals((T::CollectionSeries, K::CollectionId)),
    );
    restrict_series(query, scope);
}

/// Joins the series of the `COLLECTION_SERIES` rows in `query` and keeps those in `scope`, if
/// any.
fn restrict_series(query: &mut SelectStatement, scope: Option<&ContentScope>) {
    query.left_join(
        S::Table,
        Expr::col((T::CollectionSeries, K::SeriesId)).equals((S::Table, S::Id)),
    );
    if let Some(scope) = scope {
        keep_series_in(query, scope);
    }
}

/// Keeps the rows of the series in `query` that `scope` allows.
fn keep_series_in(query: &mut SelectStatement, scope: &ContentScope) {
    if let Some(condition) = content_restrictions::condition(scope.restrictions()) {
        query
            .left_join(
                SD::Table,
                Expr::col((SD::Table, SD::SeriesId)).equals((S::Table, S::Id)),
            )
            .cond_where(condition);
    }
    if let Some(ids) = scope.library_ids() {
        query.and_where(Expr::col((S::Table, S::LibraryId)).is_in(ids));
    }
}
