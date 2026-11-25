//! REST `BookController` routes (`/api/v1/books/…`) and shared book download/page handlers.

mod catalog;
mod import;
mod metadata;
pub(crate) mod pages;
pub(crate) mod read_progress;
mod thumbnails;
mod webpub;

use axum::Router;
use axum::extract::{OriginalUri, Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use komga_service::{BookAnalysisService, ReadingService};
use komga_types::BookId;
use serde::Deserialize;

use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::media::file_response;
use crate::mvc::produces::{Controller, JSON, OCTET_STREAM};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/books/{book_id}/analyze", post(analyze))
        // Spring's `file/*` also matches an empty last segment, but not further segments.
        .route_producing("/api/v1/books/{book_id}/file", OCTET_STREAM, get(download))
        .route_producing("/api/v1/books/{book_id}/file/", OCTET_STREAM, get(download))
        .route_producing(
            "/api/v1/books/{book_id}/file/{name}",
            OCTET_STREAM,
            get(download),
        )
        .into_router()
        .merge(catalog::routes())
        .merge(import::routes())
        .merge(metadata::routes())
        .merge(pages::routes())
        .merge(read_progress::routes())
        .merge(thumbnails::routes())
        .merge(webpub::routes())
}

pub(super) fn public_routes() -> Router<ApiState> {
    webpub::public_routes()
}

async fn analyze(
    Admin(user): Admin,
    State(analysis): State<BookAnalysisService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    analysis
        .submit(&user, &id)
        .await
        .map_err(|error| ApiError::from_book_analysis(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
pub(crate) struct BookIdParam {
    book_id: BookId,
}

/// The whole file, streamed: no ranges, no validators and no ETag, as in Kotlin.
pub(crate) async fn download(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path(BookIdParam { book_id }): Path<BookIdParam>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let file = reading
        .book_file(&user, &book_id)
        .await
        .map_err(|error| ApiError::from_reading(&error, uri.path()))?;
    Ok(file_response(file))
}
