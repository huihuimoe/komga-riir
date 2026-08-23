use komga_application::task_processing::{TaskExecutionResult, TaskQueueRecord};

#[path = "runtime/context.rs"]
mod runtime_context;
pub use runtime_context::{
    DatabaseRuntime, FilesystemRuntime, JobRuntime, SearchRuntime, TaskRuntimeConfig,
    TaskRuntimeContext, TaskRuntimeOwnershipOverrides, WorkerRuntime,
};
mod dispatch;
mod enqueue_adapter;
#[path = "runtime/execution_loop.rs"]
mod execution_loop;
#[path = "runtime/execution_pool.rs"]
mod execution_pool;
mod jobs;
pub(crate) mod queue;
#[path = "queue/store.rs"]
mod queue_core;
#[path = "queue/orchestration.rs"]
mod queue_orchestration;
#[path = "queue/scheduler.rs"]
pub(crate) mod queue_scheduler;
#[path = "runtime/engine.rs"]
mod runtime_task_engine;
#[cfg(test)]
pub(crate) mod test_support;
#[path = "runtime/workers.rs"]
mod worker_runtime;

pub use crate::media::library_scan::SqliteFilesystemLibraryScanPipeline;
pub use enqueue_adapter::TaskEnqueueAdapter;
pub use queue_scheduler::TaskQueueScheduler;
pub use worker_runtime::{
    RuntimeBackgroundState, SharedTaskQueue, TaskQueueWakeSignal,
    cleanup_authentication_activity_once, prepare_task_queue, process_startup_library_scans,
    run_background_task_iteration, run_periodic_library_scan_iteration,
};
