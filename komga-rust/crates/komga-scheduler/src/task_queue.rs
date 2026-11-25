//! Kotlin `TaskEmitter.submitTask` and the `TasksRepository` operations workers use.

use std::collections::BTreeMap;
use std::slice;
use std::sync::Arc;
use std::time::Duration;

use komga_db::{Database, DbError, DbTransaction, TaskEnd, TaskHandoverDao, TaskRow, TasksDao};
use tokio::sync::{Mutex, Notify};
use tracing::{error, info, warn};

use crate::metrics::TaskMetrics;
use crate::task::{SUPPORTED_CLASSES, Task, TaskDecodeError};

/// Executions a task gets before a retryable failure or an interruption makes it fail.
pub(crate) const MAX_ATTEMPTS: i32 = 3;

/// The persistent queue in the tasks database, plus the in-process wake-up signal for workers.
#[derive(Clone)]
pub struct TaskQueue {
    db: Database,
    wake: Arc<Notify>,
    metrics: TaskMetrics,
    /// One hand-over at a time, so that a staged task is not moved twice.
    handover: Arc<Mutex<()>>,
}

/// A claimed task; its payload may fail to decode.
pub(crate) struct Claimed {
    pub id: String,
    pub version: i32,
    pub attempts: i32,
    pub task: Result<Task, TaskDecodeError>,
}

impl TaskQueue {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self {
            db,
            wake: Arc::new(Notify::new()),
            metrics: TaskMetrics::default(),
            handover: Arc::default(),
        }
    }

    /// What the workers taking from this queue executed.
    #[must_use]
    pub fn metrics(&self) -> &TaskMetrics {
        &self.metrics
    }

    /// Commits the task before waking workers: the wake-up is only a hint, and a task committed
    /// before a crash is still queued on restart.
    ///
    /// # Errors
    /// Returns the tasks database failure; the task is then not queued.
    pub async fn submit(&self, task: &Task) -> Result<(), DbError> {
        self.submit_all(slice::from_ref(task)).await
    }

    /// Queues the task unless one with its unique ID is queued, running or failed, so repeated
    /// requests for the same work neither restart it nor retry a failure. Returns whether it was
    /// queued.
    ///
    /// # Errors
    /// Returns the tasks database failure; the task is then not queued.
    pub async fn submit_if_absent(&self, task: &Task) -> Result<bool, DbError> {
        let mut tx = self.db.begin_immediate().await?;
        let inserted = TasksDao::insert_if_absent(&mut tx, &task.to_row()).await?;
        tx.commit().await?;
        if inserted {
            self.wake.notify_one();
        }
        Ok(inserted)
    }

    /// Deletes queued tasks and returns their count; see [`TasksDao::delete_queued`].
    ///
    /// # Errors
    /// Returns the tasks database failure; the queue is then unchanged.
    pub async fn clear(&self) -> Result<u64, DbError> {
        let mut tx = self.db.begin_immediate().await?;
        let deleted = TasksDao::delete_queued(&mut tx).await?;
        tx.commit().await?;
        Ok(deleted)
    }

    /// Commits all tasks in one transaction, then wakes workers; see [`TaskQueue::submit`].
    ///
    /// # Errors
    /// Returns the tasks database failure; no task is then queued.
    pub async fn submit_all(&self, tasks: &[Task]) -> Result<(), DbError> {
        let rows: Vec<TaskRow> = tasks.iter().map(Task::to_row).collect();
        self.submit_rows(&rows).await
    }

    /// Records `tasks` in a main database transaction, so that they are queued exactly when the
    /// changes calling for them commit; [`TaskQueue::hand_over`] then moves them to the queue.
    /// The tasks database shares no transaction with the main one.
    ///
    /// # Errors
    /// Returns the main database failure.
    pub async fn stage(transaction: &mut DbTransaction, tasks: &[Task]) -> Result<(), DbError> {
        let rows: Vec<TaskRow> = tasks.iter().map(Task::to_row).collect();
        TaskHandoverDao::stage(transaction, &rows).await
    }

    /// Queues the tasks staged in committed transactions of `main`, then forgets them there. A
    /// crash in between queues them again at the next hand-over, which tasks tolerate since they
    /// run at least once anyway.
    ///
    /// The changes calling for the tasks are committed already, so a failure is only logged: the
    /// tasks stay staged for the next hand-over, at the latest at the next start.
    pub async fn hand_over(&self, main: &Database) {
        if let Err(error) = self.try_hand_over(main).await {
            error!(%error, "Failed to queue the staged tasks, they stay staged");
        }
    }

    async fn try_hand_over(&self, main: &Database) -> Result<(), DbError> {
        let _handover = self.handover.lock().await;
        let mut tx = main.begin().await?;
        let staged = TaskHandoverDao::staged(&mut tx).await?;
        tx.rollback().await?;
        let Some(&(last, _)) = staged.last() else {
            return Ok(());
        };
        let rows: Vec<TaskRow> = staged.into_iter().map(|(_, row)| row).collect();
        self.submit_rows(&rows).await?;
        let mut tx = main.begin_immediate().await?;
        TaskHandoverDao::remove_through(&mut tx, last).await?;
        tx.commit().await
    }

    async fn submit_rows(&self, tasks: &[TaskRow]) -> Result<(), DbError> {
        if tasks.is_empty() {
            return Ok(());
        }
        let mut tx = self.db.begin_immediate().await?;
        for task in tasks {
            TasksDao::save(&mut tx, task).await?;
        }
        tx.commit().await?;
        if tasks.len() == 1 {
            self.wake.notify_one();
        } else {
            self.wake.notify_waiters();
        }
        Ok(())
    }

    pub(crate) fn wake(&self) -> &Notify {
        &self.wake
    }

    /// Releases tasks left running by a previous process.
    pub(crate) async fn disown(&self) -> Result<(), DbError> {
        let mut tx = self.db.begin_immediate().await?;
        let (released, failed) = TasksDao::disown(&mut tx, MAX_ATTEMPTS).await?;
        tx.commit().await?;
        if released > 0 {
            info!(released, "Reset tasks that were not finished");
        }
        if failed > 0 {
            warn!(failed, "Failed interrupted tasks that used all attempts");
        }
        Ok(())
    }

    pub(crate) async fn take(&self, owner: &str) -> Result<Option<Claimed>, DbError> {
        let mut tx = self.db.begin_immediate().await?;
        let claimed = TasksDao::take_first(&mut tx, owner, SUPPORTED_CLASSES).await?;
        tx.commit().await?;
        Ok(claimed.map(|row| Claimed {
            task: Task::decode(&row.class, &row.payload),
            id: row.id,
            version: row.version,
            attempts: row.attempts,
        }))
    }

    /// The tasks still to run, by Kotlin simple type.
    pub async fn count_by_type(&self) -> Result<BTreeMap<String, i64>, DbError> {
        let mut tx = self.db.begin().await?;
        let counts = TasksDao::count_by_simple_type(&mut tx).await?;
        tx.rollback().await?;
        Ok(counts)
    }

    /// Time until the earliest waiting retry is due.
    pub(crate) async fn next_retry(&self) -> Result<Option<Duration>, DbError> {
        let mut tx = self.db.begin().await?;
        let seconds = TasksDao::next_retry_in(&mut tx).await?;
        tx.rollback().await?;
        Ok(seconds.map(|seconds| Duration::from_secs_f64(seconds.max(0.0))))
    }

    /// Records the end of a claimed task and wakes a worker waiting for its group.
    pub(crate) async fn finish(&self, claimed: &Claimed, end: TaskEnd<'_>) -> Result<(), DbError> {
        let mut tx = self.db.begin_immediate().await?;
        TasksDao::finish(&mut tx, &claimed.id, claimed.version, end).await?;
        tx.commit().await?;
        self.wake.notify_one();
        Ok(())
    }
}
