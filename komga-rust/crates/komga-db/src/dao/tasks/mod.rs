//! Kotlin `TasksDao`: the persistent task queue in the tasks database.
//!
//! The `TASK` schema stays identical to Kotlin's so the original server can still open the
//! database. Retry and recovery state lives in `era*` payload fields, which Kotlin's Jackson
//! mapper ignores: `eraVersion` (bumped on each resubmission), `eraAttempts`, `eraRetryAt`,
//! `eraFailed`, and `eraFailure`.

use std::collections::BTreeMap;

use crate::iden::TaskTable as T;
use sea_query::{Expr, ExprTrait, OnConflict, Order, Query, SimpleExpr, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::{AssertSqlSafe, Row};

use crate::{DbError, DbTransaction};

/// A `TASK` row; `class` and `payload` keep the Kotlin Jackson encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub id: String,
    pub priority: i32,
    pub group_id: Option<String>,
    pub class: String,
    pub simple_type: String,
    pub payload: String,
}

/// A task taken by a worker. `version` changes when the task is submitted again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedTask {
    pub id: String,
    pub class: String,
    pub payload: String,
    pub version: i32,
    pub attempts: i32,
}

/// How a claimed task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEnd<'a> {
    Succeeded,
    Retry {
        delay_seconds: i64,
        failure: &'a str,
    },
    Failed {
        failure: &'a str,
    },
    /// Stopped for a shutdown: queued again, and the attempt it was claimed for is returned.
    Interrupted,
}

pub struct TasksDao;

const VERSION: &str = "COALESCE(json_extract(PAYLOAD, '$.eraVersion'), 0)";
const ATTEMPTS: &str = "COALESCE(json_extract(PAYLOAD, '$.eraAttempts'), 0)";
const RETRY_AT: &str = "json_extract(PAYLOAD, '$.eraRetryAt')";
/// Retry times keep milliseconds: whole seconds would cut up to a second off each backoff.
const NOW_MS: &str = "strftime('%Y-%m-%d %H:%M:%f', 'now')";
const FAILED: &str = "COALESCE(json_extract(PAYLOAD, '$.eraFailed'), 0)";

impl TasksDao {
    /// Inserts the task or, when its unique ID exists already, replaces its content, resets its
    /// retry state, and moves it behind tasks of the same priority. A running task keeps its owner
    /// and runs again after it ends.
    pub async fn save(transaction: &mut DbTransaction, task: &TaskRow) -> Result<(), DbError> {
        let conflict = OnConflict::column(T::Id)
            .update_columns([T::GroupId, T::Priority, T::Class, T::SimpleType])
            .value(
                T::Payload,
                Expr::cust(
                    "json_set(excluded.PAYLOAD, '$.eraVersion', \
                     COALESCE(json_extract(TASK.PAYLOAD, '$.eraVersion'), 0) + 1)",
                ),
            )
            .value(T::LastModifiedDate, Expr::current_timestamp())
            .to_owned();
        Self::insert(transaction, task, conflict).await.map(|_| ())
    }

    /// Inserts the task unless a task with its unique ID exists, queued, running or failed.
    /// Returns whether it was inserted.
    pub async fn insert_if_absent(
        transaction: &mut DbTransaction,
        task: &TaskRow,
    ) -> Result<bool, DbError> {
        let conflict = OnConflict::column(T::Id).do_nothing().to_owned();
        Ok(Self::insert(transaction, task, conflict).await? > 0)
    }

    async fn insert(
        transaction: &mut DbTransaction,
        task: &TaskRow,
        conflict: OnConflict,
    ) -> Result<u64, DbError> {
        let (sql, values) = Query::insert()
            .into_table(T::Table)
            .columns([
                T::Id,
                T::Priority,
                T::GroupId,
                T::Class,
                T::SimpleType,
                T::Payload,
            ])
            .values_panic([
                task.id.as_str().into(),
                task.priority.into(),
                task.group_id.clone().into(),
                task.class.as_str().into(),
                task.simple_type.as_str().into(),
                task.payload.as_str().into(),
            ])
            .on_conflict(conflict)
            .build_sqlx(SqliteQueryBuilder);
        let result = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(result.rows_affected())
    }

    /// Claims the first unowned, unfailed, due task of a known class whose group has no owned
    /// task, by highest priority and then least recently modified, and counts the attempt. Rows of
    /// other classes stay queued untouched.
    ///
    /// Callers use an immediate transaction so concurrent workers cannot claim the same row.
    pub async fn take_first(
        transaction: &mut DbTransaction,
        owner: &str,
        classes: &[&str],
    ) -> Result<Option<ClaimedTask>, DbError> {
        let busy_groups = Query::select()
            .column(T::GroupId)
            .from(T::Table)
            .and_where(Expr::col(T::Owner).is_not_null())
            .and_where(Expr::col(T::GroupId).is_not_null())
            .take();
        let (sql, values) = Query::select()
            .columns([T::Id, T::Class, T::Payload])
            .expr(Expr::cust(VERSION))
            .expr(Expr::cust(ATTEMPTS))
            .from(T::Table)
            .and_where(Expr::col(T::Owner).is_null())
            .and_where(Expr::col(T::Class).is_in(classes.iter().copied()))
            .and_where(
                Expr::col(T::GroupId)
                    .is_null()
                    .or(Expr::col(T::GroupId).not_in_subquery(busy_groups)),
            )
            .and_where(pending())
            .and_where(Expr::cust(format!(
                "({RETRY_AT} IS NULL OR {RETRY_AT} <= {NOW_MS})"
            )))
            .order_by(T::Priority, Order::Desc)
            .order_by(T::LastModifiedDate, Order::Asc)
            // Timestamps have second precision; the rowid keeps submission order among ties.
            .order_by_expr(Expr::cust("rowid"), Order::Asc)
            .limit(1)
            .build_sqlx(SqliteQueryBuilder);
        let Some(row) = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?
        else {
            return Ok(None);
        };
        let task = ClaimedTask {
            id: row.try_get(0)?,
            class: row.try_get(1)?,
            payload: row.try_get(2)?,
            version: row.try_get(3)?,
            attempts: row.try_get::<i32, _>(4)? + 1,
        };
        let (sql, values) = Query::update()
            .table(T::Table)
            .value(T::Owner, owner)
            .value(
                T::Payload,
                Expr::cust_with_values("json_set(PAYLOAD, '$.eraAttempts', ?)", [task.attempts]),
            )
            .and_where(Expr::col(T::Id).eq(task.id.as_str()))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(Some(task))
    }

    pub async fn count(transaction: &mut DbTransaction) -> Result<i64, DbError> {
        let (sql, values) = Query::select()
            .expr(Expr::col(T::Id).count())
            .from(T::Table)
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?)
    }

    /// Kotlin `countBySimpleType`: the tasks still to run, by type.
    pub async fn count_by_simple_type(
        transaction: &mut DbTransaction,
    ) -> Result<BTreeMap<String, i64>, DbError> {
        let (sql, values) = Query::select()
            .column(T::SimpleType)
            .expr(Expr::col(T::Id).count())
            .from(T::Table)
            .and_where(pending())
            .group_by_col(T::SimpleType)
            .build_sqlx(SqliteQueryBuilder);
        let counts: Vec<(String, i64)> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        Ok(counts.into_iter().collect())
    }

    /// Records how a claimed task ended. If it was submitted again meanwhile, only the claim is
    /// released so that the new submission runs.
    pub async fn finish(
        transaction: &mut DbTransaction,
        id: &str,
        version: i32,
        end: TaskEnd<'_>,
    ) -> Result<(), DbError> {
        let claimed = Expr::col(T::Id)
            .eq(id)
            .and(Expr::cust_with_values(format!("{VERSION} = ?"), [version]));
        let ended = match end {
            TaskEnd::Succeeded => Query::delete()
                .from_table(T::Table)
                .cond_where(claimed)
                .build_sqlx(SqliteQueryBuilder),
            TaskEnd::Retry {
                delay_seconds,
                failure,
            } => Query::update()
                .table(T::Table)
                .value(
                    T::Payload,
                    Expr::cust_with_values(
                        "json_set(PAYLOAD, '$.eraRetryAt', strftime('%Y-%m-%d %H:%M:%f', 'now', ?), '$.eraFailure', ?)",
                        [format!("+{delay_seconds} seconds"), failure.to_owned()],
                    ),
                )
                .cond_where(claimed)
                .build_sqlx(SqliteQueryBuilder),
            TaskEnd::Failed { failure } => Query::update()
                .table(T::Table)
                .value(T::Payload, failed_payload(failure))
                .cond_where(claimed)
                .build_sqlx(SqliteQueryBuilder),
            TaskEnd::Interrupted => Query::update()
                .table(T::Table)
                .value(
                    T::Payload,
                    Expr::cust(format!(
                        "json_set(PAYLOAD, '$.eraAttempts', {ATTEMPTS} - 1)"
                    )),
                )
                .cond_where(claimed)
                .build_sqlx(SqliteQueryBuilder),
        };
        let released = Query::update()
            .table(T::Table)
            .value(T::Owner, Option::<String>::None)
            .and_where(Expr::col(T::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        for (sql, values) in [ended, released] {
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }

    /// `disown` at startup: releases tasks a previous process left running. Those that used
    /// `max_attempts` already fail instead, so a task that kills the process cannot loop forever.
    /// Returns the released and failed counts.
    pub async fn disown(
        transaction: &mut DbTransaction,
        max_attempts: i32,
    ) -> Result<(u64, u64), DbError> {
        let failure = format!("Interrupted by a shutdown or crash after {max_attempts} attempts");
        let exhausted = Query::update()
            .table(T::Table)
            .value(T::Owner, Option::<String>::None)
            .value(T::Payload, failed_payload(&failure))
            .and_where(Expr::col(T::Owner).is_not_null())
            .and_where(Expr::cust_with_values(
                format!("{ATTEMPTS} >= ?"),
                [max_attempts],
            ))
            .build_sqlx(SqliteQueryBuilder);
        let released = Query::update()
            .table(T::Table)
            .value(T::Owner, Option::<String>::None)
            .and_where(Expr::col(T::Owner).is_not_null())
            .build_sqlx(SqliteQueryBuilder);
        let mut counts = Vec::with_capacity(2);
        for (sql, values) in [exhausted, released] {
            let result = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
            counts.push(result.rows_affected());
        }
        let (failed, released) = (counts[0], counts[1]);
        Ok((released, failed))
    }

    /// `deleteAllWithoutOwner`: deletes queued tasks, including those waiting for a retry, and
    /// returns their count. Running tasks continue; failed tasks stay recorded.
    pub async fn delete_queued(transaction: &mut DbTransaction) -> Result<u64, DbError> {
        let (sql, values) = Query::delete()
            .from_table(T::Table)
            .and_where(Expr::col(T::Owner).is_null())
            .and_where(pending())
            .build_sqlx(SqliteQueryBuilder);
        let result = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(result.rows_affected())
    }

    /// Seconds until the earliest pending retry is due; `None` when no retry is waiting.
    pub async fn next_retry_in(transaction: &mut DbTransaction) -> Result<Option<f64>, DbError> {
        let (sql, values) = Query::select()
            .expr(Expr::cust(format!(
                "MIN(unixepoch({RETRY_AT}, 'subsec') - unixepoch('now', 'subsec'))"
            )))
            .from(T::Table)
            .and_where(Expr::col(T::Owner).is_null())
            .and_where(pending())
            .and_where(Expr::cust(format!("{RETRY_AT} IS NOT NULL")))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_one(transaction.connection())
            .await?)
    }
}

fn pending() -> SimpleExpr {
    Expr::cust(format!("{FAILED} = 0"))
}

fn failed_payload(failure: &str) -> SimpleExpr {
    Expr::cust_with_values(
        "json_set(PAYLOAD, '$.eraFailed', 1, '$.eraFailure', ?)",
        [failure.to_owned()],
    )
}

#[cfg(test)]
mod tests;
