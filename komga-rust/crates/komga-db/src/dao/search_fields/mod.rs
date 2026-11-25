//! What the search index reads of books and series, a batch of entities at a time.

use std::collections::HashMap;

use komga_types::BookAuthor;
use sea_query::{Expr, ExprTrait, Iden, IntoIden, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::{DbError, DbTransaction};

mod book;
mod series;

pub use book::{BookSearchFields, OneshotSeriesFields};
pub use series::SeriesSearchFields;

/// The `[name, role]` authors of `table` by `owner`, in table order.
async fn find_authors<I: Iden + 'static>(
    transaction: &mut DbTransaction,
    table: I,
    owner: I,
    [name, role]: [I; 2],
    owners: &[&str],
) -> Result<HashMap<String, Vec<BookAuthor>>, DbError> {
    let mut authors: HashMap<String, Vec<BookAuthor>> = HashMap::new();
    if owners.is_empty() {
        return Ok(authors);
    }
    let owner = owner.into_iden();
    let (sql, values) = Query::select()
        .columns([owner.clone(), name.into_iden(), role.into_iden()])
        .from(table)
        .and_where(Expr::col(owner).is_in(owners.iter().copied()))
        .build_sqlx(SqliteQueryBuilder);
    let rows: Vec<(String, String, String)> =
        sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
    for (owner, name, role) in rows {
        authors
            .entry(owner)
            .or_default()
            .push(BookAuthor { name, role });
    }
    Ok(authors)
}

/// The `column` values of `table` by `owner`, in table order.
async fn find_values<I: Iden + 'static>(
    transaction: &mut DbTransaction,
    table: I,
    owner: I,
    column: I,
    owners: &[&str],
) -> Result<HashMap<String, Vec<String>>, DbError> {
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    if owners.is_empty() {
        return Ok(grouped);
    }
    let owner = owner.into_iden();
    let (sql, values) = Query::select()
        .columns([owner.clone(), column.into_iden()])
        .from(table)
        .and_where(Expr::col(owner).is_in(owners.iter().copied()))
        .build_sqlx(SqliteQueryBuilder);
    let rows: Vec<(String, String)> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_all(transaction.connection())
        .await?;
    for (owner, value) in rows {
        grouped.entry(owner).or_default().push(value);
    }
    Ok(grouped)
}
