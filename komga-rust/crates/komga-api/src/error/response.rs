//! HTTP error bodies: Spring Boot's default JSON error attributes with messages enabled, the
//! baseline `ValidationErrorResponse` for Bean Validation failures, and Spring `ProblemDetail`.

use axum::Json;
use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use jiff::Timestamp;
use serde::Serialize;

use super::{ApiError, Violation};
use crate::accept;

#[derive(Serialize)]
struct ErrorBody {
    timestamp: Timestamp,
    status: u16,
    error: String,
    message: String,
    path: String,
}

/// Spring `ProblemDetail` in Jackson's field order.
#[derive(Serialize)]
struct ProblemDetail {
    #[serde(rename = "type")]
    kind: &'static str,
    title: &'static str,
    status: u16,
    detail: String,
    instance: String,
}

#[derive(Serialize)]
struct ValidationErrorResponse {
    violations: Vec<Violation>,
}

/// Marks the body Spring's error controller writes after the `/error` dispatch.
#[derive(Clone, Copy)]
struct SpringErrorBody;

/// Captured `Accept` header values without heap allocation for the common 0- or 1-header case.
enum AcceptHeaders {
    None,
    One(HeaderValue),
    Many(Vec<HeaderValue>),
}

impl AcceptHeaders {
    fn from_request(request: &Request) -> Self {
        let mut values = request.headers().get_all(ACCEPT).iter();
        let Some(first) = values.next() else {
            return Self::None;
        };
        let Some(second) = values.next() else {
            return Self::One(first.clone());
        };
        let mut all = Vec::with_capacity(2 + values.size_hint().0);
        all.push(first.clone());
        all.push(second.clone());
        all.extend(values.cloned());
        Self::Many(all)
    }

    fn as_slice(&self) -> &[HeaderValue] {
        match self {
            Self::None => &[],
            Self::One(value) => std::slice::from_ref(value),
            Self::Many(values) => values.as_slice(),
        }
    }
}

/// Spring's error controller writes its body only as a JSON type the request accepts; for any
/// other `Accept`, an unparseable one included, the body is empty. Kotlin also writes XML and
/// HTML error pages, which rust leaves empty.
pub(crate) async fn negotiate(request: Request, next: Next) -> Response {
    let accept = AcceptHeaders::from_request(&request);
    let mut response = next.run(request).await;
    if response.extensions().get::<SpringErrorBody>().is_none() {
        return response;
    }
    let json_type = accept::parse_values(accept.as_slice())
        .ok()
        .and_then(accept::error_json_type);
    match json_type {
        Some(json_type) if json_type != "application/json" => {
            let json_type = HeaderValue::from_str(&json_type).expect("Media types are ASCII");
            response.headers_mut().insert(CONTENT_TYPE, json_type);
            response
        }
        Some(_) => response,
        None => {
            let (mut parts, _) = response.into_parts();
            parts.headers.remove(CONTENT_TYPE);
            parts.headers.remove(CONTENT_LENGTH);
            Response::from_parts(parts, Body::empty())
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Spring {
                status,
                message,
                path,
            } => {
                let body = ErrorBody {
                    timestamp: Timestamp::now(),
                    status: status.as_u16(),
                    error: status.canonical_reason().unwrap_or("Unknown").to_owned(),
                    message,
                    path,
                };
                let mut response = (status, Json(body)).into_response();
                response.extensions_mut().insert(SpringErrorBody);
                response
            }
            Self::Validation(violations) => (
                StatusCode::BAD_REQUEST,
                Json(ValidationErrorResponse { violations }),
            )
                .into_response(),
            Self::Problem {
                status,
                detail,
                path,
            } => {
                let body = ProblemDetail {
                    kind: "about:blank",
                    title: status.canonical_reason().unwrap_or("Unknown"),
                    status: status.as_u16(),
                    detail,
                    instance: path,
                };
                let mut response = (status, Json(body)).into_response();
                response.headers_mut().insert(
                    CONTENT_TYPE,
                    HeaderValue::from_static("application/problem+json"),
                );
                response
            }
        }
    }
}
