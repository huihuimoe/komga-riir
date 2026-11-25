//! The REST WebPub routes of `BookController` (`/api/v1/books/{book_id}/…`).

use axum::extract::{OriginalUri, Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use komga_service::ReadingService;
use komga_types::BookId;

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::not_modified;
use crate::mvc::produces::{ALL, Controller, JSON};
use crate::rest::dto::R2Positions;
use crate::routes::ApiState;
use crate::webpub::handlers::{
    divina_manifest, empty_resource, epub_manifest, manifest, pdf_manifest, resource,
};

const POSITION_LIST_JSON: HeaderValue =
    HeaderValue::from_static("application/vnd.readium.position-list+json");

const MANIFEST: &[&str] = &["application/webpub+json", "application/divina+json"];
const WEBPUB: &[&str] = &["application/webpub+json"];
const DIVINA: &[&str] = &["application/divina+json"];
const POSITION_LIST: &[&str] = &["application/vnd.readium.position-list+json"];

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route_producing("/api/v1/books/{book_id}/manifest", MANIFEST, get(manifest))
        .route_producing(
            "/api/v1/books/{book_id}/manifest/epub",
            WEBPUB,
            get(epub_manifest),
        )
        .route_producing(
            "/api/v1/books/{book_id}/manifest/pdf",
            WEBPUB,
            get(pdf_manifest),
        )
        .route_producing(
            "/api/v1/books/{book_id}/manifest/divina",
            DIVINA,
            get(divina_manifest),
        )
        .route_producing(
            "/api/v1/books/{book_id}/positions",
            POSITION_LIST,
            get(positions),
        )
        .into_router()
}

/// Fonts are requested anonymously; the handler authorizes everything else.
pub(super) fn public_routes() -> Router<ApiState> {
    // Spring's `{*resource}` also matches no segment, with or without the slash.
    Controller::new(ALL)
        .route("/api/v1/books/{book_id}/resource", get(empty_resource))
        .route("/api/v1/books/{book_id}/resource/", get(empty_resource))
        .route(
            "/api/v1/books/{book_id}/resource/{*resource}",
            get(resource),
        )
        .into_router()
}

/// `getBookPositions`: the 304 check precedes the restriction check.
async fn positions(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path(id): Path<BookId>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let reading_error = |error| ApiError::from_reading(&error, uri.path());
    let source = reading.book_source(&id).await.map_err(reading_error)?;
    let last_modified = source.last_modified();
    if not_modified::check(&headers, last_modified) {
        return Ok(not_modified::response(last_modified));
    }
    let positions = reading
        .positions(&user, source)
        .await
        .map_err(reading_error)?;
    Ok((
        not_modified::cache_headers(last_modified),
        [(CONTENT_TYPE, POSITION_LIST_JSON)],
        Json(R2Positions::new(positions)),
    )
        .into_response())
}
