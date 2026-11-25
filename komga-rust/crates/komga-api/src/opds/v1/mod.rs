//! Kotlin `OpdsController`: the OPDS 1.2 catalog.

mod catalog;
mod entries;
mod request;

use axum::Router;
use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use komga_service::{ArtworkService, ReadingService};
use komga_types::BookId;

use super::OpdsState;
use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::media::{PageQuery, jpeg, page_internal};
use crate::mvc::produces::{Controller, JPEG, OCTET_STREAM};
use crate::mvc::request_params::{self, RequestParams};
use crate::rest::book::download;

pub(super) fn routes() -> Router<OpdsState> {
    Controller::new(request::PRODUCES)
        .route_producing(
            "/opds/v1.2/books/{book_id}/file/",
            OCTET_STREAM,
            get(download),
        )
        .route_producing(
            "/opds/v1.2/books/{book_id}/file/{name}",
            OCTET_STREAM,
            get(download),
        )
        .route_producing(
            "/opds/v1.2/books/{book_id}/thumbnail/small",
            JPEG,
            get(thumbnail_small),
        )
        .route_producing(
            "/opds/v1.2/books/{book_id}/pages/{page_number}",
            &["image/png", "image/gif", "image/jpeg"],
            get(page),
        )
        .route("/opds/v1.2/catalog", get(catalog::catalog))
        .route("/opds/v1.2/search", get(catalog::search))
        .route("/opds/v1.2/ondeck", get(catalog::on_deck))
        .route("/opds/v1.2/keep-reading", get(catalog::keep_reading))
        .route("/opds/v1.2/series", get(catalog::series))
        .route("/opds/v1.2/series/latest", get(catalog::latest_series))
        .route("/opds/v1.2/series/{series_id}", get(catalog::one_series))
        .route("/opds/v1.2/books/latest", get(catalog::latest_books))
        .route("/opds/v1.2/libraries", get(catalog::libraries))
        .route("/opds/v1.2/libraries/{library_id}", get(catalog::library))
        .route("/opds/v1.2/collections", get(catalog::collections))
        .route(
            "/opds/v1.2/collections/{collection_id}",
            get(catalog::collection),
        )
        .route("/opds/v1.2/readlists", get(catalog::read_lists))
        .route(
            "/opds/v1.2/readlists/{read_list_id}",
            get(catalog::read_list),
        )
        .route("/opds/v1.2/publishers", get(catalog::publishers))
        .into_router()
}

async fn thumbnail_small(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let bytes = artwork
        .book_thumbnail_small(&user, &id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(jpeg(bytes))
}

/// The page streaming link counts pages from 0.
async fn page(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path((id, number)): Path<(BookId, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let path = uri.path();
    let number = request_params::int_path_variable("pageNumber", &number)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let convert = RequestParams::parse(query.as_deref()).joined("convert");
    let number = number.wrapping_add(1);
    page_internal(
        &user,
        &reading,
        PageQuery {
            id: &id,
            number,
            convert: convert.as_deref(),
            accept: None,
        },
        &headers,
        path,
    )
    .await
}
