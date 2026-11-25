//! Authentication state and REST session routes.

mod cookies;
pub mod extractors;
mod middleware;
mod oauth2;
pub mod request_metadata;
mod resolver;

use std::sync::Arc;

use axum::Router;
use axum::extract::{Extension, OriginalUri, State};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use cookie::Cookie;
use komga_service::{IdentityService, RememberMeSettings, SessionRegistry};
use komga_types::KomgaUser;

use self::cookies::{
    append_expired_remember_me_cookie, append_session_cookie, is_xml_http_request,
    session_cookie_path,
};
pub(crate) use self::middleware::{
    authenticate_kobo, authenticate_koreader, authenticate_request, permit_anonymous,
};
use self::resolver::extract_session_token;
use crate::accept;
use crate::error::ApiError;
use crate::mvc::http_cache;
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::servlet_uri::RequestContext;
use crate::routes::ApiState;

pub const SESSION_COOKIE_NAME: &str = "KOMGA-SESSION";
pub const REMEMBER_ME_COOKIE_NAME: &str = "komga-remember-me";

/// Shared state for authentication routes and extractors.
#[derive(Clone)]
pub struct AuthState(Arc<AuthStateInner>);

struct AuthStateInner {
    identity: IdentityService,
    sessions: SessionRegistry,
    remember_me_settings: RememberMeSettings,
}

impl AuthState {
    #[must_use]
    pub fn new(
        identity: IdentityService,
        sessions: SessionRegistry,
        remember_me_settings: RememberMeSettings,
    ) -> Self {
        Self(Arc::new(AuthStateInner {
            identity,
            sessions,
            remember_me_settings,
        }))
    }

    pub(crate) fn identity(&self) -> &IdentityService {
        &self.0.identity
    }

    pub(crate) fn sessions(&self) -> &SessionRegistry {
        &self.0.sessions
    }

    pub(crate) fn remember_me_settings(&self) -> &RememberMeSettings {
        &self.0.remember_me_settings
    }
}

#[derive(Clone)]
pub(crate) struct RequestAuthContext {
    pub(crate) user: Arc<KomgaUser>,
    pub(crate) session_token: Option<String>,
}

/// Create router for logout and session conversion.
pub fn router(state: &ApiState) -> Router {
    let auth = state.auth.clone();
    let authenticated_routes = Controller::new(JSON)
        .route("/api/v1/login/set-cookie", get(set_cookie))
        .into_router()
        .route_layer(from_fn_with_state(auth.clone(), authenticate_request));

    Router::new()
        .route("/api/logout", any(logout))
        .merge(authenticated_routes)
        .with_state(auth)
        .merge(oauth2::router(state))
}

async fn logout(
    State(state): State<AuthState>,
    context: RequestContext,
    headers: HeaderMap,
) -> Response {
    if let Some(token) = extract_session_token(&headers) {
        state.sessions().invalidate(&token);
    }

    let expired_session = Cookie::build((SESSION_COOKIE_NAME, ""))
        .path(session_cookie_path(context.path()))
        .http_only(true)
        .secure(context.secure())
        .max_age(cookie::time::Duration::ZERO)
        .build();

    // `HttpBasicConfigurer` registers its 204 for the requests its entry point challenges; the
    // others get `LogoutConfigurer`'s redirect to the login page (relative path avoids open
    // redirects if `Host` is untrusted).
    let mut response = if is_xml_http_request(&headers) || accept::prefers_basic_challenge(&headers)
    {
        StatusCode::NO_CONTENT.into_response()
    } else {
        let location = format!("{}/login?logout", context.path());
        let location =
            HeaderValue::from_str(&location).expect("The context path is a valid header value");
        (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
    };
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&expired_session.to_string())
            .expect("expired session cookie serialization produces a valid header value"),
    );
    append_expired_remember_me_cookie(&mut response, context.secure(), context.path());

    // Spring Security's `LogoutFilter` answers before any handler.
    http_cache::before_handler(response)
}

async fn set_cookie(
    State(state): State<AuthState>,
    Extension(auth): Extension<RequestAuthContext>,
    context: RequestContext,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let Some(token) = auth.session_token else {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "Session required",
            uri.path(),
        ));
    };

    let mut response = StatusCode::NO_CONTENT.into_response();
    append_session_cookie(
        &mut response,
        &token,
        state.sessions().ttl(),
        context.secure(),
        context.path(),
    );
    Ok(response)
}
