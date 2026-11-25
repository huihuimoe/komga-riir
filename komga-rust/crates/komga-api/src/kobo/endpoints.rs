//! Device, initialization, book metadata, cover, and file endpoints for Kobo.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::header::{CONTENT_TYPE, LOCATION};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use komga_service::KoboReply;
use komga_types::{BookId, ThumbnailId};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tracing::warn;

use super::dto::{AuthDto, KoboBookMetadataDto};
use super::proxy::StoreError;
use super::{KoboState, device_url, from_kobo, settings, to_store};
use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::media::file_response;
use crate::mvc::servlet_uri::{RequestContext, encode_path_segment};

/// The store's `Resources` when it is not asked.
const NATIVE_RESOURCES: &str = include_str!("native_resources.json");
/// `KoboProxy.imageHostUrl`.
const STORE_IMAGE_URL: &str = "https://cdn.kobo.com/book-images";

#[derive(Deserialize)]
pub(super) struct CoverPath {
    thumbnail_id: ThumbnailId,
    width: String,
    height: String,
}

#[derive(Deserialize)]
pub(super) struct FileQuery {
    #[serde(default)]
    convert_kepub: bool,
}

/// Spring writes the `String` as is, under the controller's JSON content type.
pub(super) async fn ping() -> Response {
    ([(CONTENT_TYPE, "application/json")], "pong").into_response()
}

pub(super) async fn initialization(
    State(state): State<KoboState>,
    Path(key): Path<String>,
    headers: HeaderMap,
    context: RequestContext,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let settings = settings(&state, &uri).await?;
    let store_resources = if settings.proxy {
        store_resources(&state, &uri, &headers).await?
    } else {
        None
    };
    let context = device_url(&state, settings, &context);
    let mut resources = store_resources.unwrap_or_else(|| {
        serde_json::from_str(NATIVE_RESOURCES).expect("The native resources are a JSON object")
    });
    let images = format!(
        "{context}/kobo/{}/v1/books/{{ImageId}}/thumbnail",
        encode_path_segment(&key)
    );
    resources.insert("image_host".to_owned(), json!(context));
    resources.insert(
        "image_url_template".to_owned(),
        json!(format!("{images}/{{Width}}/{{Height}}/false/image.jpg")),
    );
    resources.insert(
        "image_url_quality_template".to_owned(),
        json!(format!(
            "{images}/{{Width}}/{{Height}}/{{Quality}}/{{IsGreyscale}}/image.jpg"
        )),
    );
    Ok((
        [("x-kobo-apitoken", "e30=")],
        Json(json!({ "Resources": resources })),
    )
        .into_response())
}

/// The store's `Resources`; `None` falls back to the native ones, except that the store refusing
/// the device's credentials is the device's answer.
async fn store_resources(
    state: &KoboState,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<Option<Map<String, Value>>, ApiError> {
    match state
        .store
        .forward(Method::GET, uri, headers, Bytes::new(), None)
        .await
    {
        Ok(reply) => Ok(reply
            .body
            .and_then(|mut body| match body.get_mut("Resources")?.take() {
                Value::Object(resources) => Some(resources),
                _ => None,
            })),
        Err(error @ StoreError::Status(StatusCode::UNAUTHORIZED)) => {
            Err(error.into_api_error(uri.path()))
        }
        Err(error) => {
            warn!(%error, "Failed to get response from Kobo /v1/initialization, fallback to noproxy");
            Ok(None)
        }
    }
}

/// The store authenticates the device when its proxy is on; otherwise, or when the store fails,
/// dummy tokens keep the device going.
pub(super) async fn auth_device(
    State(state): State<KoboState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Response {
    if state
        .kobo
        .settings()
        .await
        .is_ok_and(|settings| settings.proxy)
    {
        match state
            .store
            .forward(Method::POST, &uri, &headers, body.clone(), None)
            .await
        {
            Ok(reply) => return reply.into_response(),
            Err(error) => {
                warn!(%error, "Failed to get response from Kobo /v1/auth/device, fallback to noproxy");
            }
        }
    }
    let user_key = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|body| body.get("UserKey")?.as_str().map(str::to_owned))
        .unwrap_or_default();
    Json(AuthDto {
        access_token: random_alphanumeric(24),
        refresh_token: random_alphanumeric(24),
        token_type: "Bearer",
        tracking_id: random_uuid(),
        user_key,
    })
    .into_response()
}

pub(super) async fn book_metadata(
    State(state): State<KoboState>,
    Authenticated(user): Authenticated,
    Path((key, book_id)): Path<(String, BookId)>,
    headers: HeaderMap,
    context: RequestContext,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let books = match state.kobo.book_metadata(&user, &book_id).await {
        Ok(KoboReply::Local(books)) => books,
        Ok(KoboReply::Store) => {
            return to_store(&state, Method::GET, &uri, &headers, Bytes::new()).await;
        }
        Err(error) => return Err(from_kobo(&error, uri.path())),
    };
    let settings = settings(&state, &uri).await?;
    let context = device_url(&state, settings, &context);
    let key = encode_path_segment(&key);
    let books: Vec<_> = books
        .into_iter()
        .map(|book| {
            let url = format!("{context}/kobo/{key}/v1/books/{}/file/epub", book.id);
            KoboBookMetadataDto::new(book, &url)
        })
        .collect();
    Ok(Json(books).into_response())
}

/// The size asked for is ignored: the device scales the thumbnail.
pub(super) async fn cover(
    State(state): State<KoboState>,
    Authenticated(user): Authenticated,
    Path(CoverPath {
        thumbnail_id: id,
        width,
        height,
    }): Path<CoverPath>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    match state.kobo.cover(&user, &id).await {
        Ok(KoboReply::Local(jpeg)) => Ok(([(CONTENT_TYPE, "image/jpeg")], jpeg).into_response()),
        Ok(KoboReply::Store) => {
            if !is_safe_path_segment(id.as_str())
                || !is_safe_path_segment(&width)
                || !is_safe_path_segment(&height)
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "400 BAD_REQUEST",
                    uri.path(),
                ));
            }
            let location = format!(
                "{STORE_IMAGE_URL}/{}/{}/{}/false/image.jpg",
                encode_path_segment(id.as_str()),
                encode_path_segment(&width),
                encode_path_segment(&height)
            );
            Ok((StatusCode::TEMPORARY_REDIRECT, [(LOCATION, location)]).into_response())
        }
        Err(error) => Err(from_kobo(&error, uri.path())),
    }
}

/// The book's file, or its KEPUB form, converted in full before the response starts so that a
/// failed conversion is an error rather than a cut-off download.
pub(super) async fn book_file(
    Authenticated(user): Authenticated,
    State(state): State<KoboState>,
    Path((_, book_id)): Path<(String, BookId)>,
    Query(FileQuery { convert_kepub }): Query<FileQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let file = state
        .reading
        .kobo_book_file(&user, &book_id, convert_kepub)
        .await
        .map_err(|error| ApiError::from_reading(&error, uri.path()))?;
    Ok(file_response(file))
}

/// Rejects empty segments, dot segments, and path separators to prevent redirect path traversal.
fn is_safe_path_segment(segment: &str) -> bool {
    !segment.is_empty() && segment != "." && segment != ".." && !segment.contains(['/', '\\'])
}

/// Kotlin `RandomStringUtils.secure().nextAlphanumeric`.
fn random_alphanumeric(length: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut token = String::with_capacity(length);
    let mut byte = [0u8];
    while token.len() < length {
        getrandom::fill(&mut byte).expect("Failed to get random bytes");
        // 248 is the largest multiple of 62 below 256: rejecting above it keeps the draw uniform.
        if byte[0] < 248 {
            token.push(char::from(ALPHABET[usize::from(byte[0] % 62)]));
        }
    }
    token
}

/// A version 4 UUID, as `UUID.randomUUID`.
fn random_uuid() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("Failed to get random bytes");
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::is_safe_path_segment;

    #[test]
    fn rejects_traversal_and_separator_segments() {
        assert!(is_safe_path_segment("abc-123"));
        assert!(is_safe_path_segment("300"));
        for bad in ["", ".", "..", "../secret", "a/b", r"a\b"] {
            assert!(!is_safe_path_segment(bad), "{bad:?}");
        }
    }
}
