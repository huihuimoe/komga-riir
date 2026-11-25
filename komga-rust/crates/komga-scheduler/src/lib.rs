//! Persistent task queue, claiming, deduplication, and worker execution for Komga.

mod metrics;
mod scheduler;
mod task;
mod task_queue;

pub use metrics::{TaskMetrics, TaskStats};
pub use scheduler::{Scheduler, TaskExecutor, TaskFailure, TaskPoolSize};
pub use task::{
    DEFAULT_PRIORITY, HIGH_PRIORITY, HIGHEST_PRIORITY, LOW_PRIORITY, LOWEST_PRIORITY, Task,
    TaskKind,
};
pub use task_queue::TaskQueue;
