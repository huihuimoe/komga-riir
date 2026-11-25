//! TaskController routes.

use axum::extract::{OriginalUri, State};
use axum::routing::delete;
use axum::{Json, Router};
use komga_service::TaskService;

use crate::auth::extractors::Admin;
use crate::error::ApiError;
use crate::mvc::produces::{Controller, JSON};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/tasks", delete(clear_queue))
        .into_router()
}

async fn clear_queue(
    Admin(user): Admin,
    State(tasks): State<TaskService>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<u64>, ApiError> {
    tasks
        .clear_queue(&user)
        .await
        .map(Json)
        .map_err(|error| ApiError::from_task(&error, uri.path()))
}
