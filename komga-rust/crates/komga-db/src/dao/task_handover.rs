//! `ERA_TASK_HANDOVER`: tasks committed in the main database, to be moved to the tasks database.

use sea_query::{Expr, ExprTrait, Order, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::{AssertSqlSafe, Row};

use crate::iden::TaskHandoverTable as T;
use crate::{DbError, DbTransaction, TaskRow};

pub struct TaskHandoverDao;

impl TaskHandoverDao {
    /// Records `tasks` in the main database transaction that calls for them.
    pub async fn stage(transaction: &mut DbTransaction, tasks: &[TaskRow]) -> Result<(), DbError> {
        for task in tasks {
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
                .build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }

    /// The staged tasks in staging order, each with its sequence number.
    pub async fn staged(transaction: &mut DbTransaction) -> Result<Vec<(i64, TaskRow)>, DbError> {
        let (sql, values) = Query::select()
            .columns([
                T::Seq,
                T::Id,
                T::Priority,
                T::GroupId,
                T::Class,
                T::SimpleType,
                T::Payload,
            ])
            .from(T::Table)
            .order_by(T::Seq, Order::Asc)
            .build_sqlx(SqliteQueryBuilder);
        let rows = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.iter()
            .map(|row| {
                let task = TaskRow {
                    id: row.try_get(1)?,
                    priority: row.try_get(2)?,
                    group_id: row.try_get(3)?,
                    class: row.try_get(4)?,
                    simple_type: row.try_get(5)?,
                    payload: row.try_get(6)?,
                };
                Ok((row.try_get(0)?, task))
            })
            .collect()
    }

    /// Removes the staged tasks up to `seq`, once they are in the tasks database.
    pub async fn remove_through(transaction: &mut DbTransaction, seq: i64) -> Result<(), DbError> {
        let (sql, values) = Query::delete()
            .from_table(T::Table)
            .and_where(Expr::col(T::Seq).lte(seq))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }
}
