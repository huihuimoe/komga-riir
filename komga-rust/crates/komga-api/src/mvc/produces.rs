//! Spring's `produces` condition: a route whose types no `Accept` range accepts answers 406 Not
//! Acceptable before its arguments are bound.

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::MethodRouter;

use super::http_cache::before_handler;
use super::multipart_form;
use crate::accept;
use crate::error::ApiError;

pub(crate) const JSON: &[&str] = &["application/json"];
pub(crate) const JPEG: &[&str] = &["image/jpeg"];
pub(crate) const OCTET_STREAM: &[&str] = &["application/octet-stream"];
pub(crate) const ALL: &[&str] = &["*/*"];

/// The routes of one controller, each with the controller's `produces` unless it overrides it.
pub(crate) struct Controller<S> {
    router: Router<S>,
    produces: &'static [&'static str],
}

impl<S: Clone + Send + Sync + 'static> Controller<S> {
    pub(crate) fn new(produces: &'static [&'static str]) -> Self {
        Self {
            router: Router::new(),
            produces,
        }
    }

    pub(crate) fn route(self, path: &str, method_router: MethodRouter<S>) -> Self {
        let produces = self.produces;
        self.route_producing(path, produces, method_router)
    }

    /// A route with its own `produces`. A path can take several, one per method.
    pub(crate) fn route_producing(
        mut self,
        path: &str,
        produces: &'static [&'static str],
        method_router: MethodRouter<S>,
    ) -> Self {
        self.router = self.router.route(path, producing(produces, method_router));
        self
    }

    /// A `consumes = multipart/form-data` route: Spring checks the content type before
    /// `produces`.
    pub(crate) fn route_multipart(mut self, path: &str, method_router: MethodRouter<S>) -> Self {
        let method_router = producing(self.produces, method_router)
            .route_layer(middleware::from_fn(multipart_form::require_multipart))
            .layer(DefaultBodyLimit::max(multipart_form::MAX_REQUEST_BYTES));
        self.router = self.router.route(path, method_router);
        self
    }

    pub(crate) fn into_router(self) -> Router<S> {
        self.router
    }
}

fn producing<S: Clone + Send + Sync + 'static>(
    produces: &'static [&'static str],
    method_router: MethodRouter<S>,
) -> MethodRouter<S> {
    // `MethodRouter::route_layer` leaves out the 405 fallback, which Spring answers first.
    method_router.route_layer(middleware::from_fn(move |request: Request, next: Next| {
        negotiate(produces, request, next)
    }))
}

/// Runs the route when the request accepts one of `produces`, and gives a JSON body the type
/// Spring's message converter would have written.
pub(crate) async fn negotiate(
    produces: &'static [&'static str],
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    let Some(selected) = accept::parse(headers)
        .ok()
        .filter(|ranges| {
            produces
                .iter()
                .any(|media_type| accept::accepts(ranges, media_type))
        })
        .and_then(|ranges| accept::produced(ranges, produces))
    else {
        return not_acceptable(produces, request.uri().path());
    };
    let response = next.run(request).await;
    written_as(selected, response)
}

/// Handlers write JSON as `application/json`; Spring writes the type it selected from
/// `produces`, such as Kobo's `charset=utf-8` or the actuator's own type. Errors are written by
/// Spring's error controller as `application/json`.
pub(crate) fn written_as(selected: &str, mut response: Response) -> Response {
    let json = HeaderValue::from_static("application/json");
    if response.status().is_success()
        && response.headers().get(CONTENT_TYPE) == Some(&json)
        && !selected.contains('*')
    {
        let selected = HeaderValue::from_str(selected).expect("Media types are ASCII");
        response.headers_mut().insert(CONTENT_TYPE, selected);
    }
    response
}

/// Whether the `Accept` ranges, when Spring can parse them, accept one of `produces`.
pub(crate) fn accepts(headers: &HeaderMap, produces: &[&str]) -> bool {
    accept::parse(headers).is_ok_and(|ranges| {
        produces
            .iter()
            .any(|media_type| accept::accepts(&ranges, media_type))
    })
}

/// `HttpMediaTypeNotAcceptableException` thrown while matching the routes of the path, which
/// together produce `produces`: the types in `Accept`.
pub(crate) fn not_acceptable(produces: &[&str], path: &str) -> Response {
    let produced = produces.join(", ");
    let message = format!("Acceptable representations: [{produced}].");
    let mut response = ApiError::new(StatusCode::NOT_ACCEPTABLE, message, path).into_response();
    let produced = HeaderValue::from_str(&produced).expect("Media types are ASCII");
    response.headers_mut().insert(ACCEPT, produced);
    before_handler(response)
}
