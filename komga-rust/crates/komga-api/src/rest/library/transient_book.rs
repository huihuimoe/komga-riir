//! TransientBooksController: admins review books outside every library before importing them.

use axum::extract::{OriginalUri, Path, State};
use axum::http::HeaderMap;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use komga_service::TransientBookService;
use komga_types::BookId;
use serde::Deserialize;

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::media::media_type_or_default;
use crate::mvc::produces::{ALL, Controller, JSON};
use crate::mvc::{not_modified, request_params};
use crate::rest::dto::TransientBookDto;
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/transient-books", post(scan))
        .route("/api/v1/transient-books/{id}/analyze", post(analyze))
        .route_producing(
            "/api/v1/transient-books/{id}/pages/{pageNumber}",
            ALL,
            get(page),
        )
        .into_router()
}

#[derive(Deserialize)]
struct ScanRequestDto {
    path: String,
}

async fn scan(
    Authenticated(user): Authenticated,
    State(transient_books): State<TransientBookService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(request): JsonBody<ScanRequestDto>,
) -> Result<Json<Vec<TransientBookDto>>, ApiError> {
    let books = transient_books
        .scan(&user, request.path)
        .await
        .map_err(|error| ApiError::from_transient_book(&error, uri.path()))?;
    Ok(Json(books.into_iter().map(Into::into).collect()))
}

async fn analyze(
    Authenticated(user): Authenticated,
    State(transient_books): State<TransientBookService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<TransientBookDto>, ApiError> {
    let book = transient_books
        .analyze(&user, &id)
        .await
        .map_err(|error| ApiError::from_transient_book(&error, uri.path()))?;
    Ok(Json(book.into()))
}

async fn page(
    Authenticated(user): Authenticated,
    State(transient_books): State<TransientBookService>,
    Path((id, number)): Path<(BookId, String)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let path = uri.path();
    let number = request_params::int_path_variable("pageNumber", &number)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let book = transient_books
        .find(&user, &id)
        .map_err(|error| ApiError::from_transient_book(&error, path))?;
    let last_modified = book.last_modified;
    if not_modified::check(&headers, last_modified) {
        return Ok(not_modified::response(last_modified));
    }
    let content = transient_books
        .read_page(book, number)
        .await
        .map_err(|error| ApiError::from_reading(&error, path))?;
    Ok((
        not_modified::cache_headers(last_modified),
        [(
            CONTENT_TYPE,
            media_type_or_default(Some(&content.media_type)),
        )],
        content.bytes,
    )
        .into_response())
}
