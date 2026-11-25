//! Required and optional `@RequestBody` binding with Spring MVC's message-reading outcomes.

use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{FromRequest, OptionalFromRequest, Request};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;

use crate::error::ApiError;

/// Types the baseline's Jackson JSON converter reads. Its Jackson XML converter is not ported.
const SUPPORTED_MEDIA_TYPES: HeaderValue =
    HeaderValue::from_static("application/json, application/*+json");
const ACCEPT_PATCH: HeaderName = HeaderName::from_static("accept-patch");

/// A JSON request body; `Option<JsonBody<T>>` permits an empty body or JSON null.
pub(crate) struct JsonBody<T>(pub(crate) T);

pub(crate) enum JsonBodyRejection {
    Body(BytesRejection),
    Api(ApiError),
    Unsupported { patch: bool, error: ApiError },
}

impl IntoResponse for JsonBodyRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Body(error) => error.into_response(),
            Self::Api(error) => error.into_response(),
            Self::Unsupported { patch, error } => {
                let mut response = error.into_response();
                let headers = response.headers_mut();
                headers.insert(ACCEPT, SUPPORTED_MEDIA_TYPES);
                if patch {
                    headers.insert(ACCEPT_PATCH, SUPPORTED_MEDIA_TYPES);
                }
                response
            }
        }
    }
}

impl<T, S> FromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = JsonBodyRejection;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (path, body) = read_body(request, state).await?;
        body.map(Self)
            .ok_or_else(|| bad_request("Required request body is missing", &path))
    }
}

impl<T, S> OptionalFromRequest<S> for JsonBody<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = JsonBodyRejection;

    async fn from_request(request: Request, state: &S) -> Result<Option<Self>, Self::Rejection> {
        let (_, body) = read_body::<Option<T>, _>(request, state).await?;
        Ok(body.flatten().map(Self))
    }
}

/// No `Content-Type` reads as octet-stream unless the body is empty too.
async fn read_body<T, S>(
    request: Request,
    state: &S,
) -> Result<(String, Option<T>), JsonBodyRejection>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    let content_type = match request.headers().get(CONTENT_TYPE) {
        Some(value) => match media_type(value) {
            Some(media_type) => Some(media_type),
            None => return Err(unsupported(&method, "Could not parse Content-Type.", &path)),
        },
        None => None,
    };
    let body = Bytes::from_request(request, state)
        .await
        .map_err(JsonBodyRejection::Body)?;
    let readable = content_type.as_deref().is_some_and(is_json);
    if body.is_empty() && (readable || content_type.is_none()) {
        return Ok((path, None));
    }
    if !readable {
        let media_type = content_type
            .as_deref()
            .unwrap_or("application/octet-stream");
        let detail = format!("Content-Type '{media_type}' is not supported.");
        return Err(unsupported(&method, &detail, &path));
    }
    let body = serde_json::from_slice(&body)
        .map_err(|error| bad_request(&format!("JSON parse error: {error}"), &path))?;
    Ok((path, Some(body)))
}

/// Spring `MediaType.toString()` form: lowercase type and subtype, `;`-joined parameters.
pub(crate) fn media_type(value: &HeaderValue) -> Option<String> {
    let mut parts = value.to_str().ok()?.split(';');
    let essence = parts.next()?.trim().to_ascii_lowercase();
    let (kind, subtype) = essence.split_once('/')?;
    if kind.is_empty() || subtype.is_empty() {
        return None;
    }
    let mut media_type = essence.clone();
    for parameter in parts
        .map(str::trim)
        .filter(|parameter| !parameter.is_empty())
    {
        let (name, value) = parameter.split_once('=')?;
        media_type.push_str(&format!(";{}={}", name.trim(), value.trim()));
    }
    Some(media_type)
}

fn is_json(media_type: &str) -> bool {
    let essence = media_type.split(';').next().unwrap_or_default();
    essence
        .strip_prefix("application/")
        .is_some_and(|subtype| subtype == "json" || subtype.ends_with("+json"))
}

fn bad_request(message: &str, path: &str) -> JsonBodyRejection {
    JsonBodyRejection::Api(ApiError::new(StatusCode::BAD_REQUEST, message, path))
}

/// `HttpMediaTypeNotSupportedException`: 415 with its detail, advertising readable types.
fn unsupported(method: &Method, detail: &str, path: &str) -> JsonBodyRejection {
    JsonBodyRejection::Unsupported {
        patch: method == Method::PATCH,
        error: ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, detail, path),
    }
}
