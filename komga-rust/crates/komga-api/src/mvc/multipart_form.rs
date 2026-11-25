//! `consumes = multipart/form-data` routes (`Controller::route_multipart`): Spring MVC's handler
//! mapping by content type, and the eager multipart resolution that reads every part, with Spring
//! Boot's default size limits, before the arguments bind and `@PreAuthorize` runs.

use axum::extract::multipart::MultipartError;
use axum::extract::{FromRequest, Multipart, Request};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;
use crate::mvc::request_params::RequestParams;
use crate::mvc::{http_cache, json_body};

/// Spring Boot's default `spring.servlet.multipart.max-request-size`.
pub(crate) const MAX_REQUEST_BYTES: usize = 10 * 1024 * 1024;
/// Spring Boot's default `spring.servlet.multipart.max-file-size`, which the baseline does not
/// override and komga-rust does not make configurable. Tomcat applies it to every part, form
/// fields included.
pub(crate) const MAX_FILE_BYTES: usize = 1024 * 1024;

/// The request parameters, query string first and then the form fields, and the file parts.
pub(crate) struct MultipartForm {
    params: RequestParams,
    files: Vec<(String, Vec<u8>)>,
}

impl MultipartForm {
    pub(crate) fn params(&self) -> &RequestParams {
        &self.params
    }

    /// A required `MultipartFile` parameter: the first file part with this name, else
    /// `MissingServletRequestPartException`.
    pub(crate) fn required_file(&self, name: &str, path: &str) -> Result<&[u8], ApiError> {
        self.files
            .iter()
            .find(|(file_name, _)| file_name == name)
            .map(|(_, bytes)| bytes.as_slice())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Required part '{name}' is not present."),
                    path,
                )
            })
    }
}

impl<S> FromRequest<S> for MultipartForm
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let path = request.uri().path().to_owned();
        let mut params = RequestParams::parse(request.uri().query());
        let malformed = || {
            // Spring wraps parse failures in a `MultipartException` that no resolver handles,
            // so the baseline answers 500; a malformed body is the client's error here.
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "Failed to parse multipart servlet request",
                &path,
            )
        };
        let too_large = || {
            ApiError::problem(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Maximum upload size exceeded",
                &path,
            )
        };
        let failed = |error: MultipartError| {
            if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                too_large()
            } else {
                malformed()
            }
        };
        let mut multipart = Multipart::from_request(request, state)
            .await
            .map_err(|_| malformed())?;
        let mut files = Vec::new();
        while let Some(mut field) = multipart.next_field().await.map_err(failed)? {
            let name = field.name().unwrap_or_default().to_owned();
            let is_file = field.file_name().is_some();
            let mut bytes = Vec::new();
            while let Some(chunk) = field.chunk().await.map_err(failed)? {
                if bytes.len() + chunk.len() > MAX_FILE_BYTES {
                    return Err(too_large());
                }
                bytes.extend_from_slice(&chunk);
            }
            if is_file {
                files.push((name, bytes));
            } else {
                params.push(name, String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        Ok(Self { params, files })
    }
}

/// `RequestMappingInfoHandlerMapping.handleNoMatch` for a consumes mismatch: 415 with
/// `HttpMediaTypeNotSupportedException`'s detail, which shows a missing type as `null`.
pub(crate) async fn require_multipart(request: Request, next: Next) -> Response {
    let detail = match request
        .headers()
        .get(CONTENT_TYPE)
        .map(json_body::media_type)
    {
        None => "Content-Type 'null' is not supported.".to_owned(),
        Some(None) => "Could not parse Content-Type.".to_owned(),
        Some(Some(media_type)) if media_type.split(';').next() == Some("multipart/form-data") => {
            return next.run(request).await;
        }
        Some(Some(media_type)) => format!("Content-Type '{media_type}' is not supported."),
    };
    let error = ApiError::new(
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        detail,
        request.uri().path(),
    );
    let mut response = error.into_response();
    response
        .headers_mut()
        .insert(ACCEPT, HeaderValue::from_static("multipart/form-data"));
    http_cache::before_handler(response)
}
