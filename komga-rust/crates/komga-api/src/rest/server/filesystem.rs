//! FileSystemController directory listing.

use axum::extract::OriginalUri;
use axum::routing::post;
use axum::{Json, Router};
use komga_service::FilesystemService;

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::produces::{Controller, JSON};
use crate::rest::dto::{DirectoryListingDto, DirectoryRequestDto};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/filesystem", post(list))
        .into_router()
}

async fn list(
    Authenticated(user): Authenticated,
    OriginalUri(uri): OriginalUri,
    payload: Option<JsonBody<DirectoryRequestDto>>,
) -> Result<Json<DirectoryListingDto>, ApiError> {
    let request = payload.map(|JsonBody(request)| request).unwrap_or_default();
    let listing = FilesystemService::list(&user, request.path, request.show_files)
        .await
        .map_err(|error| ApiError::from_filesystem(error, uri.path()))?;
    Ok(Json(listing.into()))
}
