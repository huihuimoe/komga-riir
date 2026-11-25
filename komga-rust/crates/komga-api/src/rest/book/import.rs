//! BookController `importBooks`: admins queue books from outside every library into series.

use axum::Router;
use axum::extract::{OriginalUri, State};
use axum::http::StatusCode;
use axum::routing::post;
use komga_service::BookImporter;
use komga_types::{BookId, CopyMode, ImportRequest, SeriesId};
use serde::Deserialize;

use crate::auth::extractors::Admin;
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::produces::{Controller, JSON};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/books/import", post(import))
        .into_router()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BookImportBatchDto {
    #[serde(default)]
    books: Vec<BookImportDto>,
    copy_mode: CopyMode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BookImportDto {
    source_file: String,
    series_id: SeriesId,
    upgrade_book_id: Option<BookId>,
    destination_name: Option<String>,
}

async fn import(
    Admin(_): Admin,
    State(importer): State<BookImporter>,
    OriginalUri(uri): OriginalUri,
    JsonBody(batch): JsonBody<BookImportBatchDto>,
) -> Result<StatusCode, ApiError> {
    let requests = batch
        .books
        .into_iter()
        .map(|book| ImportRequest {
            source_file: book.source_file,
            series_id: book.series_id,
            copy_mode: batch.copy_mode,
            destination_name: book.destination_name,
            upgrade_book_id: book.upgrade_book_id,
        })
        .collect();
    importer
        .submit(requests)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(StatusCode::ACCEPTED)
}
