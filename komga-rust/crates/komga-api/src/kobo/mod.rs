//! Kotlin `KoboController`: the Kobo store API a device talks to, under `/kobo/{apiKey}/`.
//!
//! Requests Komga does not answer itself go to the Kobo store when its proxy is on, and get an
//! empty object otherwise.

mod dto;
mod endpoints;
mod library;
mod proxy;
mod sync_dto;
mod sync_token;

use std::ops::Deref;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{OriginalUri, Request, State};
use axum::handler::Handler;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter, get, post};
use axum::{Json, Router};
use komga_service::{KoboError, KoboSettings, KoboSyncService, ReadingService};
use serde_json::json;
use tracing::warn;

pub use self::proxy::KoboStore;
use crate::auth::authenticate_kobo;
use crate::error::ApiError;
use crate::mvc::produces::{self, Controller, JPEG, OCTET_STREAM};
use crate::mvc::servlet_uri::RequestContext;
use crate::routes::ApiState;

const SYNC_TOKEN: &str = "x-kobo-synctoken";
const JSON_UTF8: &[&str] = &["application/json;charset=utf-8"];
const CATCH_ALL_METHODS: [Method; 5] = [
    Method::GET,
    Method::PUT,
    Method::POST,
    Method::DELETE,
    Method::PATCH,
];

#[derive(Clone)]
struct KoboState(Arc<KoboStateInner>);

struct KoboStateInner {
    kobo: KoboSyncService,
    reading: ReadingService,
    store: KoboStore,
    /// The port the server listens on, for URLs when Kobo sets none.
    server_port: u16,
}

impl Deref for KoboState {
    type Target = KoboStateInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Create router for `/kobo/{apiKey}/…`.
pub fn router(state: &ApiState) -> Router {
    let thumbnail = "/kobo/{key}/v1/books/{thumbnail_id}/thumbnail/{width}/{height}";
    let kobo_state = KoboState(Arc::new(KoboStateInner {
        kobo: state.kobo.clone(),
        reading: state.reading.clone(),
        store: state.kobo_store.clone(),
        server_port: state.web_server.effective_port,
    }));
    Controller::new(JSON_UTF8)
        .route("/kobo/{key}/ping", get_json(endpoints::ping))
        .route(
            "/kobo/{key}/v1/initialization",
            get_json(endpoints::initialization),
        )
        .route(
            "/kobo/{key}/v1/auth/device",
            post(endpoints::auth_device).merge(catch_all_except(&[Method::POST])),
        )
        .route("/kobo/{key}/v1/library/sync", get_json(library::sync))
        .route(
            "/kobo/{key}/v1/library/{book_id}/metadata",
            get_json(endpoints::book_metadata),
        )
        .route(
            "/kobo/{key}/v1/library/{book_id}/state",
            get(library::reading_state)
                .put(library::update_reading_state)
                .merge(catch_all_except(&[Method::GET, Method::PUT])),
        )
        .route("/kobo/{key}/{*path}", catch_all_except(&[]))
        .into_router()
        .route(
            "/kobo/{key}/v1/books/{book_id}/file/epub",
            get_producing(endpoints::book_file, OCTET_STREAM, &kobo_state),
        )
        .route(
            &format!("{thumbnail}/{{grey}}/image.jpg"),
            get_producing(endpoints::cover, JPEG, &kobo_state),
        )
        .route(
            &format!("{thumbnail}/{{quality}}/{{grey}}/image.jpg"),
            get_producing(endpoints::cover, JPEG, &kobo_state),
        )
        .route_layer(from_fn_with_state(state.auth.clone(), authenticate_kobo))
        .with_state(kobo_state)
}

/// A JSON route for GET; the other methods go to the catch-all.
fn get_json<H, T>(handler: H) -> MethodRouter<KoboState>
where
    H: Handler<T, KoboState>,
    T: 'static,
{
    get(handler).merge(catch_all_except(&[Method::GET]))
}

/// A GET route that produces something else than the controller, outside it since the controller
/// would check its JSON on GET too.
fn get_producing<H, T>(
    handler: H,
    produces: &'static [&'static str],
    state: &KoboState,
) -> MethodRouter<KoboState>
where
    H: Handler<T, KoboState>,
    T: 'static,
{
    let or_catch_all = from_fn_with_state(
        state.clone(),
        move |State(state): State<KoboState>, request: Request, next: Next| {
            or_catch_all(produces, state, request, next)
        },
    );
    let catch_all_json =
        from_fn(|request: Request, next: Next| produces::negotiate(JSON_UTF8, request, next));
    get(handler)
        .route_layer(or_catch_all)
        .merge(catch_all_except(&[Method::GET]).route_layer(catch_all_json))
}

/// Spring sends the methods a Kobo route does not map to the catch-all.
fn catch_all_except(mapped: &[Method]) -> MethodRouter<KoboState> {
    CATCH_ALL_METHODS
        .into_iter()
        .filter(|method| !mapped.contains(method))
        .fold(MethodRouter::new(), |router, method| {
            let filter = MethodFilter::try_from(method).expect("A standard method");
            router.on(filter, catch_all)
        })
}

/// Spring answers with the catch-all, which produces JSON, when a route that produces something
/// else cannot.
async fn or_catch_all(
    produces: &'static [&'static str],
    state: KoboState,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();
    if produces::accepts(headers, produces) {
        next.run(request).await
    } else if produces::accepts(headers, JSON_UTF8) {
        let response = catch_all.call(request, state).await;
        produces::written_as(JSON_UTF8[0], response)
    } else {
        let produced = [produces, JSON_UTF8].concat();
        produces::not_acceptable(&produced, request.uri().path())
    }
}

async fn catch_all(
    State(state): State<KoboState>,
    method: Method,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response, ApiError> {
    if settings(&state, &uri).await?.proxy {
        to_store(&state, method, &uri, &headers, body).await
    } else {
        Ok(Json(json!({})).into_response())
    }
}

/// The store's answer to the device's request, its error status included.
async fn to_store(
    state: &KoboState,
    method: Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    match state.store.forward(method, uri, headers, body, None).await {
        Ok(reply) => Ok(reply.into_response()),
        Err(error) => {
            warn!(%error, path = uri.path(), "Kobo store request failed");
            Err(error.into_api_error(uri.path()))
        }
    }
}

fn from_kobo(error: &KoboError, path: &str) -> ApiError {
    match error {
        KoboError::NotFound => ApiError::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path),
        KoboError::Restricted => ApiError::new(StatusCode::FORBIDDEN, "403 FORBIDDEN", path),
        KoboError::BadRequest => ApiError::new(StatusCode::BAD_REQUEST, "400 BAD_REQUEST", path),
        _ => ApiError::internal_error(path, error),
    }
}

async fn settings(state: &KoboState, uri: &Uri) -> Result<KoboSettings, ApiError> {
    state
        .kobo
        .settings()
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))
}

/// The server's URL as the device reaches it.
fn device_url(state: &KoboState, settings: KoboSettings, context: &RequestContext) -> String {
    context.kobo_url(settings.port.unwrap_or(i32::from(state.server_port)))
}
