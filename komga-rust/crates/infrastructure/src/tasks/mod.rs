pub(crate) mod dispatch;
mod jobs;
mod runtime;
#[cfg(test)]
pub(crate) mod test_support;

pub use komga_infrastructure_tasks::{TaskEnqueueAdapter, TaskQueueScheduler};
pub use runtime::{
    DatabaseRuntime, FilesystemRuntime, JobRuntime, RuntimeBackgroundState, SearchRuntime,
    SharedTaskQueue, TaskQueueWakeSignal, TaskRuntimeConfig, TaskRuntimeContext,
    TaskRuntimeOwnershipOverrides, WorkerRuntime, cleanup_authentication_activity_once,
    prepare_task_queue, process_startup_library_scans, run_background_task_iteration,
    run_periodic_library_scan_iteration,
};
