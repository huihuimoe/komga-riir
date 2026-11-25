//! Kotlin `Opds2Controller` and `OpdsAuthenticationEntryPoint`: the OPDS 2 catalog.

mod books;
mod feeds;
mod request;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_TYPE, LINK, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, get};
use komga_types::LibraryId;

use super::OpdsState;
use super::dto::authentication::AuthenticationDocument;
use crate::mvc::produces::Controller;
use crate::mvc::servlet_uri::RequestContext;
use crate::webpub::{OPDS_AUTHENTICATION_JSON, OPDS_JSON};
use feeds::{BookFeed, ListFeed};
use request::Opds2;

pub(super) fn routes() -> Router<OpdsState> {
    let mut router = Controller::new(&[OPDS_JSON])
        .route("/opds/v2/catalog", get(feeds::recommended))
        .route("/opds/v2/collections/{id}", get(feeds::collection))
        .route("/opds/v2/readlists/{id}", get(feeds::read_list))
        .route("/opds/v2/series/{id}", get(feeds::series))
        .route("/opds/v2/search", get(feeds::search));
    for root in ["/opds/v2/libraries", "/opds/v2/libraries/{id}"] {
        router = router
            .route(root, get(feeds::recommended))
            .route(
                &format!("{root}/keep-reading"),
                books_feed(BookFeed::KeepReading),
            )
            .route(&format!("{root}/on-deck"), books_feed(BookFeed::OnDeck))
            .route(
                &format!("{root}/books/latest"),
                books_feed(BookFeed::LatestBooks),
            )
            .route(&format!("{root}/series/latest"), get(feeds::latest_series))
            .route(&format!("{root}/browse"), get(feeds::browse))
            .route(&format!("{root}/collections"), lists(ListFeed::Collections))
            .route(&format!("{root}/readlists"), lists(ListFeed::ReadLists));
    }
    router.into_router().merge(books::routes())
}

fn books_feed(kind: BookFeed) -> MethodRouter<OpdsState> {
    get(
        move |state: State<OpdsState>, id: Option<Path<LibraryId>>, request: Opds2| {
            feeds::books(kind, state, id, request)
        },
    )
}

fn lists(kind: ListFeed) -> MethodRouter<OpdsState> {
    get(
        move |state: State<OpdsState>, id: Option<Path<LibraryId>>, request: Opds2| {
            feeds::lists(kind, state, id, request)
        },
    )
}

/// Answered to anonymous requests too.
pub(super) fn public_routes() -> Router<OpdsState> {
    Controller::new(&[OPDS_AUTHENTICATION_JSON])
        .route("/opds/v2/auth", get(authentication_document))
        .into_router()
}

async fn authentication_document(context: RequestContext) -> Response {
    let document = AuthenticationDocument::new(&context.url());
    (
        [(
            CONTENT_TYPE,
            HeaderValue::from_static(OPDS_AUTHENTICATION_JSON),
        )],
        Json(document),
    )
        .into_response()
}

/// The 401 to `/opds/v2/` requests without valid credentials: the authentication document,
/// linked so that OPDS clients can show a login.
pub(crate) fn authentication_required(context: &RequestContext) -> Response {
    let context = context.url();
    let link = format!(
        r#"<{context}/opds/v2/auth>; rel="http://opds-spec.org/auth/document"; type="{OPDS_AUTHENTICATION_JSON}""#
    );
    let headers = [
        (
            CONTENT_TYPE,
            HeaderValue::from_static("application/opds-authentication+json;charset=UTF-8"),
        ),
        (
            WWW_AUTHENTICATE,
            HeaderValue::from_static(r#"Basic realm="Realm""#),
        ),
        (
            LINK,
            HeaderValue::from_str(&link).expect("The context URL is a valid header value"),
        ),
    ];
    let document = AuthenticationDocument::new(&context);
    (StatusCode::UNAUTHORIZED, headers, Json(document)).into_response()
}
