//! FontsController: the fonts the EPUB reader offers.

use std::borrow::Cow;

use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::Response;
use axum::routing::get;
use axum::{Json, Router};
use komga_service::{FontBytes, FontError, FontService};

use crate::error::ApiError;
use crate::mvc::content_disposition;
use crate::mvc::produces::{self, Controller, JSON};
use crate::mvc::resource::{self, Resource};
use crate::routes::ApiState;

const CSS: &[&str] = &["text/css"];

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/fonts/families", get(families))
        .into_router()
}

pub(super) fn public_routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/fonts/resource/{family}/{file}", get(file))
        .into_router()
        .route("/api/v1/fonts/resource/{family}/css", get(css))
}

/// `getFonts`.
async fn families(State(fonts): State<FontService>) -> Json<Vec<String>> {
    Json(fonts.families().map(str::to_owned).collect())
}

/// `getFontFile`: typed by its extension, whatever the controller's `produces`.
async fn file(
    State(fonts): State<FontService>,
    Path((family, name)): Path<(String, String)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let font = fonts
        .file(&family, &name)
        .await
        .map_err(|error| ApiError::from_font(&error, uri.path()))?;
    let media_type = HeaderValue::from_str(&font.media_type).expect("Extensions are ASCII");
    let resource = match font.bytes {
        FontBytes::Embedded(Cow::Borrowed(bytes)) => Resource::Memory(Bytes::from_static(bytes)),
        FontBytes::Embedded(Cow::Owned(bytes)) => Resource::Memory(Bytes::from(bytes)),
        FontBytes::File { content, length } => Resource::File { content, length },
    };
    attachment(&headers, media_type, &name, resource, uri.path()).await
}

/// `getFontFamilyAsCss`.
async fn css(
    State(fonts): State<FontService>,
    Path(family): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    // Spring serves the `{file}` route instead when only it can produce a type.
    if !produces::accepts(&headers, CSS) {
        if produces::accepts(&headers, JSON) {
            let path = Path((family, "css".to_owned()));
            return file(State(fonts), path, headers, OriginalUri(uri)).await;
        }
        let produced = [JSON[0], CSS[0]];
        return Ok(produces::not_acceptable(&produced, uri.path()));
    }
    let css = fonts
        .css(&family)
        .ok_or_else(|| ApiError::from_font(&FontError::NotFound, uri.path()))?;
    let media_type = HeaderValue::from_static("text/css");
    let resource = Resource::Memory(Bytes::from(css));
    attachment(
        &headers,
        media_type,
        &format!("{family}.css"),
        resource,
        uri.path(),
    )
    .await
}

async fn attachment(
    request: &HeaderMap,
    media_type: HeaderValue,
    file_name: &str,
    resource: Resource,
    path: &str,
) -> Result<Response, ApiError> {
    let headers = HeaderMap::from_iter([
        (CONTENT_TYPE, media_type),
        (
            CONTENT_DISPOSITION,
            content_disposition::latin1_header("attachment", file_name),
        ),
    ]);
    resource::response(request, headers, resource)
        .await
        .map_err(|error| ApiError::internal_error(path, error))
}
