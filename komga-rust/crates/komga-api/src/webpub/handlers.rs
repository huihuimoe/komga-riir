//! Shared WebPub manifest and EPUB resource handlers used by REST (`/api/v1/books/…`) and
//! OPDS 2 (`/opds/v2/books/…`).

use axum::Json;
use axum::extract::{OriginalUri, Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use komga_service::{ReadingService, is_epub_font};
use komga_types::{BookId, KomgaUser, MediaProfile};

use super::{Generator, OPDS_PUBLICATION_JSON, manifest as build_manifest, media_type};
use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::media::{media_ranges, media_type_or_default};
use crate::mvc::servlet_uri::RequestContext;
use crate::mvc::{content_disposition, not_modified};

/// Keeps scripts and plugins in book content from running in the reader.
const CONTENT_SECURITY_POLICY: (HeaderName, HeaderValue) = (
    HeaderName::from_static("content-security-policy"),
    HeaderValue::from_static("script-src 'none'; object-src 'none';"),
);

/// `getBookWebPubManifest`: the manifest of the book's profile.
pub(crate) async fn manifest(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path(id): Path<BookId>,
    context: RequestContext,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let profile = reading
        .manifest_profile(&id)
        .await
        .map_err(|error| ApiError::from_reading(&error, uri.path()))?;
    manifest_response(&reading, &user, &id, profile, &context, &uri).await
}

/// `getBookWebPubManifestEpub`.
pub(crate) async fn epub_manifest(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path(id): Path<BookId>,
    context: RequestContext,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    manifest_response(&reading, &user, &id, MediaProfile::Epub, &context, &uri).await
}

/// `getBookWebPubManifestPdf`.
pub(crate) async fn pdf_manifest(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path(id): Path<BookId>,
    context: RequestContext,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    manifest_response(&reading, &user, &id, MediaProfile::Pdf, &context, &uri).await
}

/// `getBookWebPubManifestDivina`, which any book has.
pub(crate) async fn divina_manifest(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path(id): Path<BookId>,
    context: RequestContext,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    manifest_response(&reading, &user, &id, MediaProfile::Divina, &context, &uri).await
}

/// The `manifest` kind of manifest; books of other profiles are rejected unless it is DiViNa.
/// Under `/opds/v2/` it is `OpdsGenerator`'s, answered as an OPDS publication.
async fn manifest_response(
    reading: &ReadingService,
    user: &KomgaUser,
    id: &BookId,
    manifest: MediaProfile,
    context: &RequestContext,
    uri: &Uri,
) -> Result<Response, ApiError> {
    let required = (manifest != MediaProfile::Divina).then_some(manifest);
    let source = reading
        .manifest_source(user, id, required)
        .await
        .map_err(|error| ApiError::from_reading(&error, uri.path()))?;
    let context = context.url();
    let (generator, content_type) = if uri.path().starts_with("/opds/") {
        (Generator::Opds, OPDS_PUBLICATION_JSON)
    } else {
        (Generator::WebPub, media_type(Some(manifest)))
    };
    Ok((
        [(CONTENT_TYPE, HeaderValue::from_static(content_type))],
        Json(build_manifest(generator, manifest, &source, &context)),
    )
        .into_response())
}

pub(crate) async fn empty_resource(
    user: Option<Authenticated>,
    State(reading): State<ReadingService>,
    Path(id): Path<BookId>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    epub_resource(user, &reading, id, "", &headers, uri.path()).await
}

pub(crate) async fn resource(
    user: Option<Authenticated>,
    State(reading): State<ReadingService>,
    Path((id, resource)): Path<(BookId, String)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    epub_resource(user, &reading, id, &resource, &headers, uri.path()).await
}

/// `getBookEpubResource`: the 304 check precedes the profile and restriction checks.
async fn epub_resource(
    user: Option<Authenticated>,
    reading: &ReadingService,
    id: BookId,
    name: &str,
    headers: &HeaderMap,
    path: &str,
) -> Result<Response, ApiError> {
    media_ranges(headers);
    let is_font = is_epub_font(name);
    let restricted_to = match &user {
        _ if is_font => None,
        Some(Authenticated(user)) => Some(user.as_ref()),
        // A `ResponseStatusException` from the controller, not the authentication entry point.
        None => {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "401 UNAUTHORIZED",
                path,
            ));
        }
    };
    let reading_error = |error| ApiError::from_reading(&error, path);
    let source = reading.book_source(&id).await.map_err(reading_error)?;
    let last_modified = source.last_modified();
    if not_modified::check(headers, last_modified) {
        return Ok((
            [CONTENT_SECURITY_POLICY],
            not_modified::response(last_modified),
        )
            .into_response());
    }
    let resource = reading
        .read_epub_resource(restricted_to, source, name)
        .await
        .map_err(reading_error)?;
    // `FilenameUtils.getName`.
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    Ok((
        not_modified::cache_headers(last_modified),
        [
            (
                CONTENT_TYPE,
                media_type_or_default(resource.media_type.as_deref()),
            ),
            (
                CONTENT_DISPOSITION,
                content_disposition::header("inline", file_name),
            ),
            CONTENT_SECURITY_POLICY,
        ],
        resource.bytes,
    )
        .into_response())
}
