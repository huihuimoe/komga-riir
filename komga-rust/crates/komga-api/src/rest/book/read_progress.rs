//! BookController read progress routes and the progression routes of CommonBookController.

use axum::extract::{OriginalUri, Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, put};
use axum::{Json, Router};
use komga_service::ReadProgressService;
use komga_types::BookId;

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::produces::{Controller, JSON};
use crate::rest::dto::{R2ProgressionDto, ReadProgressMark, ReadProgressUpdateDto};
use crate::routes::ApiState;

const PROGRESSION_JSON: HeaderValue =
    HeaderValue::from_static("application/vnd.readium.progression+json");

pub(crate) const PROGRESSION: &[&str] = &["application/vnd.readium.progression+json"];

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route(
            "/api/v1/books/{book_id}/read-progress",
            patch(mark).delete(delete),
        )
        .route_producing(
            "/api/v1/books/{book_id}/progression",
            PROGRESSION,
            get(progression),
        )
        .route("/api/v1/books/{book_id}/progression", put(mark_progression))
        .into_router()
}

/// The body is validated before the book is looked up.
async fn mark(
    Authenticated(user): Authenticated,
    State(read_progress): State<ReadProgressService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<ReadProgressUpdateDto>,
) -> Result<StatusCode, ApiError> {
    match payload.validate()? {
        ReadProgressMark::Completed => read_progress.mark_completed(&user, &id).await,
        ReadProgressMark::Page(page) => read_progress.mark_page(&user, &id, page).await,
    }
    .map_err(|error| ApiError::from_read_progress(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete(
    Authenticated(user): Authenticated,
    State(read_progress): State<ReadProgressService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    read_progress
        .delete(&user, &id)
        .await
        .map_err(|error| ApiError::from_read_progress(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// No content when the user has no progress on the book.
pub(crate) async fn progression(
    Authenticated(user): Authenticated,
    State(read_progress): State<ReadProgressService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let progress = read_progress
        .find(&user, &id)
        .await
        .map_err(|error| ApiError::from_read_progress(&error, uri.path()))?;
    Ok(match progress {
        Some(progress) => (
            [(CONTENT_TYPE, PROGRESSION_JSON)],
            Json(R2ProgressionDto::from(progress)),
        )
            .into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

pub(crate) async fn mark_progression(
    Authenticated(user): Authenticated,
    State(read_progress): State<ReadProgressService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<R2ProgressionDto>,
) -> Result<StatusCode, ApiError> {
    read_progress
        .mark_progression(&user, &id, payload.into())
        .await
        .map_err(|error| ApiError::from_read_progress(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}
