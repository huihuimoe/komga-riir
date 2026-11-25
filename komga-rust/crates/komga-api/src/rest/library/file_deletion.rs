//! BookController `deleteBookFile` and SeriesController `deleteSeriesFile`: admins queue the
//! deletion of book and series files.

use axum::Router;
use axum::extract::{OriginalUri, Path, State};
use axum::http::StatusCode;
use axum::routing::delete;
use komga_service::FileDeletion;
use komga_types::{BookId, SeriesId};

use crate::auth::extractors::Admin;
use crate::error::ApiError;
use crate::mvc::produces::{Controller, JSON};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/books/{book_id}/file", delete(delete_book_file))
        .route(
            "/api/v1/series/{series_id}/file",
            delete(delete_series_file),
        )
        .into_router()
}

async fn delete_book_file(
    Admin(_): Admin,
    State(deletion): State<FileDeletion>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    deletion
        .submit_book(&id)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(StatusCode::ACCEPTED)
}

async fn delete_series_file(
    Admin(_): Admin,
    State(deletion): State<FileDeletion>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    deletion
        .submit_series(&id)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(StatusCode::ACCEPTED)
}
