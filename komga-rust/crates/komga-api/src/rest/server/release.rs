//! `ReleaseController`.

use axum::extract::{OriginalUri, State};
use axum::routing::get;
use axum::{Json, Router};
use komga_service::{Release, ReleaseService};
use tracing::warn;

use crate::auth::extractors::Admin;
use crate::error::ApiError;
use crate::mvc::produces::{Controller, JSON};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/releases", get(list))
        .into_router()
}

async fn list(
    _admin: Admin,
    State(releases): State<ReleaseService>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<Release>>, ApiError> {
    releases.releases().await.map(Json).map_err(|error| {
        warn!(error = ?error, "Failed to get the releases");
        ApiError::internal_error(uri.path(), error)
    })
}
