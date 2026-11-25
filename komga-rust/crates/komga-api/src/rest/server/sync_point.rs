//! Sync point routes (`SyncPointController`).

use axum::Router;
use axum::extract::{OriginalUri, RawQuery, State};
use axum::http::StatusCode;
use axum::routing::delete;
use komga_service::SyncPointService;
use komga_types::ApiKeyId;

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::request_params::RequestParams;
use crate::routes::ApiState;

/// Create routes for `/api/v1/syncpoints`.
pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/syncpoints/me", delete(delete_my_sync_points))
        .into_router()
}

/// Deletes the user's sync points, or only those of the `key_id` API keys, so the next sync of
/// those clients starts from scratch.
async fn delete_my_sync_points(
    Authenticated(user): Authenticated,
    State(sync_points): State<SyncPointService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<StatusCode, ApiError> {
    let api_key_ids: Option<Vec<ApiKeyId>> = RequestParams::parse(query.as_deref())
        .list::<String>("key_id")
        .expect("Every value parses as a string")
        .filter(|ids| !ids.is_empty())
        .map(|ids| ids.into_iter().map(ApiKeyId::from).collect());
    sync_points
        .delete_for_user(&user.id, api_key_ids.as_deref())
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(StatusCode::NO_CONTENT)
}
