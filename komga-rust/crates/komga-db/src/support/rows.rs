//! Row helpers shared by the DAOs.

use std::collections::HashMap;
use std::hash::Hash;

use komga_types::{PageRequest, PageResult, SortDirection};
use sea_query::{
    Alias, Asterisk, Expr, ExprTrait, Func, Iden, IntoIden, Order, Query, SelectStatement,
    SimpleExpr, SqliteQueryBuilder, Value,
};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;
use sqlx::sqlite::SqliteRow;

use crate::{Database, DbError, DbTransaction};

/// Maximum `IN (...)` batch size per query so bound parameters stay below SQLite's 999-variable
/// default limit even when a query carries additional filters.
pub(crate) const BATCH_CHUNK_SIZE: usize = 900;

/// Deletes the rows of `table` whose `owner_column` is `owner_id`, then inserts `rows`.
pub(crate) async fn replace_rows<I, const N: usize>(
    transaction: &mut DbTransaction,
    table: I,
    owner_column: I,
    columns: [I; N],
    owner_id: impl Into<Value>,
    rows: impl Iterator<Item = [SimpleExpr; N]>,
) -> Result<(), DbError>
where
    I: Iden + 'static,
{
    let table = table.into_iden();
    let (sql, values) = Query::delete()
        .from_table(table.clone())
        .and_where(Expr::col(owner_column).eq(owner_id.into()))
        .build_sqlx(SqliteQueryBuilder);
    sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
        .execute(transaction.connection())
        .await?;
    let mut insert = Query::insert();
    insert.into_table(table).columns(columns);
    let mut any = false;
    for row in rows {
        insert.values_panic(row);
        any = true;
    }
    if any {
        let (sql, values) = insert.build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
    }
    Ok(())
}

/// The `column` values of each owner in `owner_ids`, grouped by owner in table order.
pub(crate) async fn find_values_by_owners<I, K>(
    transaction: &mut DbTransaction,
    table: I,
    owner_column: I,
    column: I,
    owner_ids: &[K],
) -> Result<HashMap<K, Vec<String>>, DbError>
where
    I: Iden + 'static,
    K: Eq + Hash + Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
    for<'a> &'a K: Into<Value>,
{
    let table = table.into_iden();
    let owner_column = owner_column.into_iden();
    let column = column.into_iden();
    let mut grouped: HashMap<K, Vec<String>> = HashMap::new();
    for chunk in owner_ids.chunks(BATCH_CHUNK_SIZE) {
        let (sql, values) = Query::select()
            .columns([owner_column.clone(), column.clone()])
            .from(table.clone())
            .and_where(Expr::col(owner_column.clone()).is_in(chunk))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<(K, String)> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        for (owner, value) in rows {
            grouped.entry(owner).or_default().push(value);
        }
    }
    Ok(grouped)
}

/// The two `columns` of each owner in `owner_ids`, grouped by owner in table order.
pub(crate) async fn find_pairs_by_owners<I, K>(
    transaction: &mut DbTransaction,
    table: I,
    owner_column: I,
    columns: [I; 2],
    owner_ids: &[K],
) -> Result<HashMap<K, Vec<(String, String)>>, DbError>
where
    I: Iden + 'static,
    K: Eq + Hash + Send + Unpin + for<'r> sqlx::Decode<'r, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite>,
    for<'a> &'a K: Into<Value>,
{
    let table = table.into_iden();
    let owner_column = owner_column.into_iden();
    let [first, second] = columns.map(IntoIden::into_iden);
    let mut grouped: HashMap<K, Vec<(String, String)>> = HashMap::new();
    for chunk in owner_ids.chunks(BATCH_CHUNK_SIZE) {
        let (sql, values) = Query::select()
            .columns([owner_column.clone(), first.clone(), second.clone()])
            .from(table.clone())
            .and_where(Expr::col(owner_column.clone()).is_in(chunk))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<(K, String, String)> =
            sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_all(transaction.connection())
                .await?;
        for (owner, left, right) in rows {
            grouped.entry(owner).or_default().push((left, right));
        }
    }
    Ok(grouped)
}

/// The requested page of `query` in `orders`, and the row count of the whole query.
pub(crate) async fn fetch_page<T>(
    database: &Database,
    query: SelectStatement,
    orders: impl Iterator<Item = (Expr, SortDirection)>,
    page: PageRequest,
    from_row: impl Fn(&SqliteRow) -> Result<T, DbError>,
) -> Result<PageResult<T>, DbError> {
    let mut page_query = query.clone();
    for (expression, direction) in orders {
        let order = match direction {
            SortDirection::Asc => Order::Asc,
            SortDirection::Desc => Order::Desc,
        };
        page_query.order_by_expr(expression, order);
    }
    if let PageRequest::Paged { size, .. } = page {
        page_query.limit(u64::from(size)).offset(page.offset());
    }
    let (sql, values) = page_query.build_sqlx(SqliteQueryBuilder);
    let rows = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_all(database.read_pool())
        .await?;
    let items = rows.iter().map(from_row).collect::<Result<Vec<T>, _>>()?;
    let total_elements = match page {
        PageRequest::Unpaged => items.len() as u64,
        PageRequest::Paged { .. } => {
            let (sql, values) = Query::select()
                .expr(Func::count(Expr::col(Asterisk)))
                .from_subquery(query, Alias::new("q"))
                .build_sqlx(SqliteQueryBuilder);
            let count: i64 = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
                .fetch_one(database.read_pool())
                .await?;
            u64::try_from(count).expect("COUNT(*) is never negative")
        }
    };
    Ok(PageResult {
        items,
        total_elements,
    })
}

/// The number of rows `query` selects.
pub(crate) async fn count(
    transaction: &mut DbTransaction,
    query: SelectStatement,
) -> Result<u64, DbError> {
    let (sql, values) = Query::select()
        .expr(Func::count(Expr::col(Asterisk)))
        .from_subquery(query, Alias::new("q"))
        .build_sqlx(SqliteQueryBuilder);
    let count: i64 = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_one(transaction.connection())
        .await?;
    Ok(u64::try_from(count).expect("COUNT(*) is never negative"))
}
