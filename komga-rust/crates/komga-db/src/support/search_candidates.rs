//! The full-text hits of one query, held in a temporary table of the transaction's connection so
//! that SQLite filters, orders, counts, and pages them with everything else (Kotlin `TempTable`).

use sea_query::{Alias, Expr, ExprTrait, IntoIden, Query, SelectStatement, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use crate::{DbError, DbTransaction};

const TABLE: &str = "ERA_SEARCH_CANDIDATE";

/// The hits written so far, ranked in the order they were written. The table exists until the
/// transaction ends: rolling back removes it, so transactions holding one never commit.
pub struct SearchCandidates {
    next_rank: i64,
}

impl SearchCandidates {
    /// Creates the empty table.
    ///
    /// # Errors
    /// Fails when the connection already holds one, or the table cannot be created.
    pub async fn create(transaction: &mut DbTransaction) -> Result<Self, DbError> {
        let sql = format!(
            "CREATE TEMPORARY TABLE {TABLE} (ID TEXT NOT NULL PRIMARY KEY, RANK INTEGER NOT NULL)"
        );
        sqlx::query(AssertSqlSafe(sql))
            .execute(transaction.connection())
            .await?;
        Ok(Self { next_rank: 0 })
    }

    /// Adds `ids`, ranked after those added before.
    ///
    /// # Errors
    /// Fails when an ID is already held, or the rows cannot be written.
    pub async fn insert(
        &mut self,
        transaction: &mut DbTransaction,
        ids: &[String],
    ) -> Result<(), DbError> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut insert = Query::insert();
        insert
            .into_table(Alias::new(TABLE))
            .columns([Alias::new("ID"), Alias::new("RANK")]);
        for id in ids {
            insert.values_panic([id.as_str().into(), self.next_rank.into()]);
            self.next_rank += 1;
        }
        let (sql, values) = insert.build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Records the size of the table for the planner, which otherwise assumes a large one and
    /// scans the whole entity table instead of looking up each hit.
    ///
    /// # Errors
    /// Fails when the statistics cannot be written.
    pub async fn analyze(&self, transaction: &mut DbTransaction) -> Result<(), DbError> {
        sqlx::query(AssertSqlSafe(format!("ANALYZE temp.{TABLE}")))
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Keeps the rows of `query` whose `id` column is a hit.
    pub(crate) fn restrict(&self, query: &mut SelectStatement, id: (impl IntoIden, impl IntoIden)) {
        query.inner_join(
            Alias::new(TABLE),
            Expr::col((Alias::new(TABLE), Alias::new("ID"))).equals(id),
        );
    }

    /// The rank of the hit of a row [`restrict`](Self::restrict) kept; the most relevant first.
    pub(crate) fn rank(&self) -> Expr {
        Expr::col((Alias::new(TABLE), Alias::new("RANK")))
    }
}
