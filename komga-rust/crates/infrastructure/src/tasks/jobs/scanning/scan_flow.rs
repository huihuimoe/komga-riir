use komga_application::task_processing::{
    ScanOneLibrary, TaskExecutionOutcome, TaskProcessingError,
};

use komga_infrastructure_media_library::library_scan::SqliteFilesystemLibraryScanPipeline;
use crate::tasks::JobRuntime;

pub(in crate::tasks) async fn execute_scan_library(
    runtime: &JobRuntime<'_>,
    request: ScanOneLibrary,
) -> Result<TaskExecutionOutcome, TaskProcessingError> {
    let cleanup_policy = runtime
        .cleanup_empty_sets_policy()
        .await
        .map_err(TaskProcessingError::runtime)?;
    let media_runtime = komga_infrastructure_media_library::MediaLibraryJobContext::new(
        runtime.database().main_db().clone(),
        runtime.database().owns_main_database(),
        runtime.filesystem().owns_filesystem_scan_output(),
        runtime.runtime_events_arc(),
    );
    let pipeline = SqliteFilesystemLibraryScanPipeline::for_runtime(&media_runtime, cleanup_policy)
        .await
        .map_err(TaskProcessingError::runtime)?;
    let result = pipeline.execute_scan(request).await?;
    Ok(TaskExecutionOutcome::with_follow_up_tasks(
        result.follow_up_tasks,
    ))
}
