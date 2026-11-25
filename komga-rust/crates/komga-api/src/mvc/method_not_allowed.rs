//! Spring MVC's answers for a mapped path that does not map the method: `OPTIONS` is 200 with
//! the allowed methods, anything else a 405 error body. Axum's own 405 has neither.

use axum::extract::{OriginalUri, Request};
use axum::http::header::{ALLOW, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;

/// Router layer: the error body, inside the error negotiation. `Allow` is added by axum later.
pub(crate) async fn error_body(request: Request, next: Next) -> Response {
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }
    let method = request.method().clone();
    // `Uri` is backed by `Bytes`, so cloning it avoids allocating a path `String` on non-405 paths.
    let uri = request
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| request.uri().clone(), |OriginalUri(uri)| uri.clone());
    let response = next.run(request).await;
    if !is_bare_405(&response) {
        return response;
    }
    let message = format!("Method '{method}' is not supported.");
    ApiError::new(StatusCode::METHOD_NOT_ALLOWED, message, uri.path()).into_response()
}

/// Around the whole router, where axum's 405 carries `Allow`.
pub(crate) async fn options(request: Request, next: Next) -> Response {
    let options = request.method() == Method::OPTIONS;
    let response = next.run(request).await;
    if !options || !is_bare_405(&response) {
        return response;
    }
    let methods = response
        .headers()
        .get(ALLOW)
        .and_then(|allow| allow.to_str().ok())
        .filter(|allow| !allow.is_empty());
    let methods = methods.map_or_else(|| "OPTIONS".to_owned(), |m| format!("{m},OPTIONS"));
    let allow = HeaderValue::from_str(&methods).expect("Allow lists header-safe tokens");
    (StatusCode::OK, [(ALLOW, allow)]).into_response()
}

fn is_bare_405(response: &Response) -> bool {
    response.status() == StatusCode::METHOD_NOT_ALLOWED
        && !response.headers().contains_key(CONTENT_TYPE)
}
