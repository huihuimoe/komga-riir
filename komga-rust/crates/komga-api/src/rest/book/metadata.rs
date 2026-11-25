//! BookController metadata routes.

use std::collections::BTreeMap;

use axum::Router;
use axum::extract::{OriginalUri, Path, State};
use axum::http::StatusCode;
use axum::routing::{patch, post};
use komga_service::MetadataService;
use komga_types::BookId;

use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::produces::{Controller, JSON};
use crate::rest::dto::BookMetadataUpdateDto;
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/books/metadata", patch(update_batch))
        .route("/api/v1/books/{book_id}/metadata", patch(update))
        .route("/api/v1/books/{book_id}/metadata/refresh", post(refresh))
        .into_router()
}

async fn refresh(
    Admin(user): Admin,
    State(metadata): State<MetadataService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    metadata
        .submit_book_refresh(&user, &id)
        .await
        .map_err(|error| ApiError::from_metadata(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

/// Spring validates the body before `@PreAuthorize` runs, so an invalid body is a 400 even for
/// non-admins.
async fn update(
    Authenticated(user): Authenticated,
    State(metadata): State<MetadataService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<BookMetadataUpdateDto>,
) -> Result<StatusCode, ApiError> {
    ApiError::check_violations(payload.violations())?;
    metadata
        .update_book_metadata(&user, &id, payload.into_update())
        .await
        .map_err(|error| ApiError::from_metadata(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Spring validates a `@Valid` map body through method validation, whose failure Komga's advice
/// does not handle: the response is Spring's default error, without violations.
async fn update_batch(
    Authenticated(user): Authenticated,
    State(metadata): State<MetadataService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<BTreeMap<String, BookMetadataUpdateDto>>,
) -> Result<StatusCode, ApiError> {
    if payload.values().any(|dto| !dto.violations().is_empty()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "Validation failure",
            uri.path(),
        ));
    }
    let updates = payload
        .into_iter()
        .map(|(id, dto)| (BookId::from(id), dto.into_update()))
        .collect();
    metadata
        .update_books_metadata(&user, updates)
        .await
        .map_err(|error| ApiError::from_metadata(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}
