use komga_application::task_processing::TaskProcessingError;

use super::super::runtime_context::JobRuntime;
use crate::search::SearchEntityType;

pub(in crate::tasks) async fn rebuild_index(
    runtime: &JobRuntime<'_>,
    entity_types: Option<&[SearchEntityType]>,
) -> Result<(), TaskProcessingError> {
    match entity_types {
        Some(entity_types) => runtime.search_engine().rebuild_entities(entity_types).await,
        None => runtime.search_engine().rebuild_all().await,
    }
    .map_err(TaskProcessingError::runtime)
}
