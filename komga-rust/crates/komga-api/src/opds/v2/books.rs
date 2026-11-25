//! OPDS 2 book routes (`/opds/v2/books/{book_id}/…`): downloads, pages, progression, WebPub
//! manifests, and EPUB resources.

use axum::Router;
use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::{get, put};
use komga_service::ReadingService;
use komga_types::BookId;

use super::OpdsState;
use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::media::{PageQuery, page_internal};
use crate::mvc::produces::{ALL, Controller, JSON, OCTET_STREAM};
use crate::mvc::request_params::{self, RequestParams};
use crate::rest::book::download;
use crate::rest::book::pages::raw_page;
use crate::rest::book::read_progress::{PROGRESSION, mark_progression, progression};
use crate::webpub::OPDS_PUBLICATION_JSON;
use crate::webpub::handlers::{
    divina_manifest, empty_resource, epub_manifest, manifest, pdf_manifest, resource,
};

const OPDS_PUBLICATION: &[&str] = &[OPDS_PUBLICATION_JSON];

pub(super) fn routes() -> Router<OpdsState> {
    Controller::new(JSON)
        .route_producing("/opds/v2/books/{book_id}/file", OCTET_STREAM, get(download))
        .route_producing(
            "/opds/v2/books/{book_id}/file/",
            OCTET_STREAM,
            get(download),
        )
        .route_producing(
            "/opds/v2/books/{book_id}/file/{name}",
            OCTET_STREAM,
            get(download),
        )
        .route_producing(
            "/opds/v2/books/{book_id}/pages/{page_number}",
            ALL,
            get(page),
        )
        .route_producing(
            "/opds/v2/books/{book_id}/pages/{page_number}/raw",
            ALL,
            get(raw_page),
        )
        .route_producing(
            "/opds/v2/books/{book_id}/progression",
            PROGRESSION,
            get(progression),
        )
        .route(
            "/opds/v2/books/{book_id}/progression",
            put(mark_progression),
        )
        .into_router()
        .merge(manifest_routes())
}

fn manifest_routes() -> Router<OpdsState> {
    Controller::new(JSON)
        .route_producing(
            "/opds/v2/books/{book_id}/manifest",
            OPDS_PUBLICATION,
            get(manifest),
        )
        .route_producing(
            "/opds/v2/books/{book_id}/manifest/epub",
            OPDS_PUBLICATION,
            get(epub_manifest),
        )
        .route_producing(
            "/opds/v2/books/{book_id}/manifest/pdf",
            OPDS_PUBLICATION,
            get(pdf_manifest),
        )
        .route_producing(
            "/opds/v2/books/{book_id}/manifest/divina",
            OPDS_PUBLICATION,
            get(divina_manifest),
        )
        .route_producing(
            "/opds/v2/books/{book_id}/resource",
            ALL,
            get(empty_resource),
        )
        .route_producing(
            "/opds/v2/books/{book_id}/resource/",
            ALL,
            get(empty_resource),
        )
        .route_producing(
            "/opds/v2/books/{book_id}/resource/{*resource}",
            ALL,
            get(resource),
        )
        .into_router()
}

/// Kotlin's page streaming route counts pages from 1, unlike OPDS 1.2's.
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
