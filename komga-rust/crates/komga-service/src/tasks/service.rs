//! Kotlin `TaskController` operations on the task queue.

use std::collections::BTreeMap;

use komga_db::DbError;
use komga_scheduler::TaskQueue;
use komga_types::KomgaUser;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TaskError {
    #[error("Forbidden")]
    Forbidden,
    #[error("Database error: {0}")]
    Db(#[from] DbError),
}

#[derive(Clone)]
pub struct TaskService {
    tasks: TaskQueue,
}

impl TaskService {
    #[must_use]
    pub fn new(tasks: TaskQueue) -> Self {
        Self { tasks }
    }

    /// `emptyTaskQueue`: cancels queued tasks and returns their count. Running tasks continue,
    /// and failed tasks stay recorded.
    pub async fn clear_queue(&self, user: &KomgaUser) -> Result<u64, TaskError> {
        if !user.is_admin() {
            return Err(TaskError::Forbidden);
        }
        Ok(self.tasks.clear().await?)
    }

    /// Pending tasks by simple type name, for administrators.
    pub async fn queue_counts(&self, user: &KomgaUser) -> Result<BTreeMap<String, i64>, TaskError> {
        if !user.is_admin() {
            return Err(TaskError::Forbidden);
        }
        Ok(self.tasks.count_by_type().await?)
    }
}
