//! Book page listing, page streaming, page thumbnail, and media-type helpers.

use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use komga_service::ReadingService;
use komga_types::BookId;

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::media::{
    PageQuery, media_ranges, media_type_or_default, page_internal, page_response,
};
use crate::mvc::not_modified;
use crate::mvc::produces::{ALL, Controller, JPEG, JSON};
use crate::mvc::request_params::{self, RequestParams};
use crate::rest::dto::PageDto;
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/books/{book_id}/pages", get(pages))
        .route_producing(
            "/api/v1/books/{book_id}/pages/{page_number}",
            ALL,
            get(page),
        )
        .route_producing(
            "/api/v1/books/{book_id}/pages/{page_number}/raw",
            ALL,
            get(raw_page),
        )
        .route_producing(
            "/api/v1/books/{book_id}/pages/{page_number}/thumbnail",
            JPEG,
            get(page_thumbnail),
        )
        .into_router()
}

async fn pages(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<PageDto>>, ApiError> {
    let pages = reading
        .pages(&user, &id)
        .await
        .map_err(|error| ApiError::from_reading(&error, uri.path()))?;
    Ok(Json(
        pages
            .into_iter()
            .enumerate()
            .map(|(index, page)| PageDto::new(index, page))
            .collect(),
    ))
}

/// Arguments bind before `@PreAuthorize` runs, so malformed ones fail before the role check.
/// A PDF page is served raw when `Accept` prefers PDF, unless `contentNegotiation` is false.
async fn page(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path((id, number)): Path<(BookId, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let accept = media_ranges(&headers);
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let mismatch = |mismatch| ApiError::type_mismatch(&mismatch, path);
    let number = request_params::int_path_variable("pageNumber", &number).map_err(mismatch)?;
    let convert = params.joined("convert");
    let zero_based = params.boolean_or("zero_based", false).map_err(mismatch)?;
    let content_negotiation = params
        .boolean_or("contentNegotiation", true)
        .map_err(mismatch)?;
    let number = if zero_based {
        number.wrapping_add(1)
    } else {
        number
    };

    let accept = content_negotiation.then_some(accept);
    page_internal(
        &user,
        &reading,
        PageQuery {
            id: &id,
            number,
            convert: convert.as_deref(),
            accept,
        },
        &headers,
        path,
    )
    .await
}

pub(crate) async fn raw_page(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path((id, number)): Path<(BookId, String)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let path = uri.path();
    let number = request_params::int_path_variable("pageNumber", &number)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let reading_error = |error| ApiError::from_reading(&error, path);
    let source = reading
        .page_source(&user, &id)
        .await
        .map_err(reading_error)?;
    let last_modified = source.last_modified();
    if not_modified::check(&headers, last_modified) {
        return Ok(not_modified::response(last_modified));
    }
    let name = format!("{}-{number}", source.book_name());
    let content = reading
        .read_raw_page(&user, source, number)
        .await
        .map_err(reading_error)?;
    Ok(page_response(&name, "", content, last_modified))
}

/// Unlike the page itself, the thumbnail needs no role and is not named.
async fn page_thumbnail(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path((id, number)): Path<(BookId, String)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let path = uri.path();
    let number = request_params::int_path_variable("pageNumber", &number)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let reading_error = |error| ApiError::from_reading(&error, path);
    let source = reading.book_source(&id).await.map_err(reading_error)?;
    let last_modified = source.last_modified();
    if not_modified::check(&headers, last_modified) {
        return Ok(not_modified::response(last_modified));
    }
    let content = reading
        .read_page_thumbnail(&user, source, number)
        .await
        .map_err(reading_error)?;
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
