//! `AnnouncementController`.

use std::collections::BTreeSet;

use axum::extract::{OriginalUri, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use komga_service::{AnnouncementError, AnnouncementService, JsonFeed};
use tracing::warn;

use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::produces::{Controller, JSON};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/announcements", get(list).put(mark_read))
        .into_router()
}

/// An empty feed is a 404, any other failure a 500.
async fn list(
    Admin(user): Admin,
    State(announcements): State<AnnouncementService>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<JsonFeed>, ApiError> {
    match announcements.announcements(user.id.as_str()).await {
        Ok(feed) => Ok(Json(feed)),
        Err(AnnouncementError::Empty) => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "404 NOT_FOUND",
            uri.path(),
        )),
        Err(error) => {
            warn!(error = ?error, "Failed to get the announcements");
            Err(ApiError::internal_error(uri.path(), error))
        }
    }
}

/// The body is read before the role is checked, as Spring does.
async fn mark_read(
    Authenticated(user): Authenticated,
    State(announcements): State<AnnouncementService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(ids): JsonBody<BTreeSet<String>>,
) -> Result<StatusCode, ApiError> {
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    announcements
        .mark_read(user.id.as_str(), &ids)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(StatusCode::NO_CONTENT)
}
