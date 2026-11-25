//! Authentication middleware for web/API, anonymous-permitted, Kobo, and KOReader routes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Query, Request, State};
use axum::http::header::{self, HeaderValue};
use axum::http::{StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use komga_types::UserRole;

use super::cookies::{
    append_expired_remember_me_cookie, append_remember_me_cookie, append_session_cookie,
    unauthorized,
};
use super::request_metadata::AuthActivityMetadata;
use super::resolver::{
    AuthSource, RequestAuthResolution, ResolvedAuth, resolve_api_key, resolve_request_auth,
};
use super::{AuthState, RequestAuthContext};
use crate::error::ApiError;
use crate::mvc::http_cache;
use crate::mvc::servlet_uri::RequestContext;

/// Whether a route admits requests without credentials (Spring Security `permitAll`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Authenticated,
    PermitAll,
}

pub(crate) async fn authenticate_request(
    State(state): State<AuthState>,
    request: Request,
    next: Next,
) -> Response {
    authenticate(&state, request, next, Access::Authenticated).await
}

/// Lets anonymous requests through; credentials that are sent must still be valid.
pub(crate) async fn permit_anonymous(
    State(state): State<AuthState>,
    request: Request,
    next: Next,
) -> Response {
    authenticate(&state, request, next, Access::PermitAll).await
}

/// Kobo's security chain: the API key in `/kobo/{key}/…` is the only credential, and the user
/// needs the `KOBO_SYNC` role.
pub(crate) async fn authenticate_kobo(
    State(state): State<AuthState>,
    request: Request,
    next: Next,
) -> Response {
    let key = kobo_api_key(request.uri().path()).map(str::to_owned);
    authenticate_device(&state, request, next, key, UserRole::KoboSync).await
}

/// KOReader's security chain: the API key in `X-Auth-User` is the only credential, and the user
/// needs the `KOREADER_SYNC` role.
pub(crate) async fn authenticate_koreader(
    State(state): State<AuthState>,
    request: Request,
    next: Next,
) -> Response {
    let key = request
        .headers()
        .get("X-Auth-User")
        .and_then(|key| key.to_str().ok())
        .map(|key| key.trim().to_owned());
    authenticate_device(&state, request, next, key, UserRole::KoreaderSync).await
}

/// A device authenticates with an API key alone. Missing or unknown keys are 401 and a missing
/// role is 403, where Spring answers both with its default 403 entry point. The key is a request
/// extension for the handlers.
async fn authenticate_device(
    state: &AuthState,
    mut request: Request,
    next: Next,
    key: Option<String>,
    role: UserRole,
) -> Response {
    let path = request.uri().path().to_owned();
    let resolution = match key.filter(|key| !key.is_empty()) {
        Some(key) => {
            let metadata = AuthActivityMetadata::from_headers(request.headers());
            resolve_api_key(state, &metadata, &key).await
        }
        None => Ok(RequestAuthResolution::Rejected),
    };
    let (user, api_key) = match resolution {
        Ok(RequestAuthResolution::Authenticated(resolved)) => (resolved.user, resolved.api_key),
        Ok(_) => {
            return ApiError::new(StatusCode::UNAUTHORIZED, "Unauthorized", &path).into_response();
        }
        Err(error) => return ApiError::internal_error(&path, error).into_response(),
    };
    if !user.has_role(role) {
        return ApiError::new(StatusCode::FORBIDDEN, "Forbidden", &path).into_response();
    }
    request.extensions_mut().insert(RequestAuthContext {
        user: Arc::new(user),
        session_token: None,
    });
    request
        .extensions_mut()
        .insert(api_key.expect("Devices authenticate with an API key"));
    next.run(request).await
}

/// The `/kobo/([\w-]+)` match of `UriRegexApiKeyAuthenticationConverter`.
fn kobo_api_key(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/kobo/")?;
    let end = rest
        .find(|character: char| !(character.is_ascii_alphanumeric() || "_-".contains(character)))
        .unwrap_or(rest.len());
    Some(&rest[..end]).filter(|key| !key.is_empty())
}

async fn authenticate(state: &AuthState, request: Request, next: Next, access: Access) -> Response {
    let context = RequestContext::from_extensions(request.extensions());
    let loaded_at = Instant::now();
    let resolution = match resolve_request_auth(state, request.headers()).await {
        Ok(resolution) => resolution,
        Err(error) => {
            return http_cache::before_handler(
                ApiError::internal_error(request.uri().path(), error).into_response(),
            );
        }
    };
    let resolved = match resolution {
        RequestAuthResolution::Authenticated(resolved) => *resolved,
        RequestAuthResolution::Anonymous if access == Access::PermitAll => {
            return next.run(request).await;
        }
        // Spring's `RememberMeAuthenticationFilter` clears a rejected cookie and continues
        // anonymously.
        RequestAuthResolution::RejectedRememberMe if access == Access::PermitAll => {
            let mut response = next.run(request).await;
            append_expired_remember_me_cookie(&mut response, context.secure(), context.path());
            return response;
        }
        RequestAuthResolution::Anonymous | RequestAuthResolution::Rejected => {
            return http_cache::before_handler(unauthorized(&request));
        }
        RequestAuthResolution::RejectedRememberMe => {
            let mut response = unauthorized(&request);
            append_expired_remember_me_cookie(&mut response, context.secure(), context.path());
            return http_cache::before_handler(response);
        }
    };
    run_authenticated(state, request, next, &context, resolved, loaded_at).await
}

async fn run_authenticated(
    state: &AuthState,
    mut request: Request,
    next: Next,
    context: &RequestContext,
    resolved: ResolvedAuth,
    loaded_at: Instant,
) -> Response {
    let uses_header_session_id = request.headers().contains_key("X-Auth-Token");
    let set_cookie_route = request.uri().path() == "/api/v1/login/set-cookie";
    let creates_session = resolved.source == AuthSource::Basic;
    let issues_remember_me_cookie = creates_session && remember_me_requested(request.uri());
    let session_token = if creates_session {
        let Some(token) = state.sessions().issue_token(&resolved.user, loaded_at) else {
            return http_cache::before_handler(unauthorized(&request));
        };
        Some(token)
    } else {
        resolved.session_token
    };
    // Only Basic authentication needs the token or user after `next.run`; avoid cloning on the
    // common session/API-key path.
    let created_session_token = if creates_session {
        session_token.clone()
    } else {
        None
    };
    let remember_me_token = issues_remember_me_cookie.then(|| {
        state
            .sessions()
            .issue_remember_me_token(&resolved.user, state.remember_me_settings())
    });
    request.extensions_mut().insert(RequestAuthContext {
        user: Arc::new(resolved.user),
        session_token,
    });

    let mut response = next.run(request).await;
    if let Some(token) = created_session_token.as_deref() {
        apply_session_headers(
            &mut response,
            state,
            context,
            token,
            remember_me_token,
            uses_header_session_id,
            set_cookie_route,
        );
    }
    response
}

fn apply_session_headers(
    response: &mut Response,
    state: &AuthState,
    context: &RequestContext,
    token: &str,
    remember_me_token: Option<String>,
    uses_header_session_id: bool,
    set_cookie_route: bool,
) {
    if !set_cookie_route {
        if uses_header_session_id {
            response.headers_mut().insert(
                header::HeaderName::from_static("x-auth-token"),
                HeaderValue::from_str(token)
                    .expect("session token contains only valid header characters"),
            );
        } else {
            append_session_cookie(
                response,
                token,
                state.sessions().ttl(),
                context.secure(),
                context.path(),
            );
        }
    }
    if let Some(remember_me_token) = remember_me_token {
        append_remember_me_cookie(
            response,
            remember_me_token,
            state.remember_me_settings(),
            context.secure(),
            context.path(),
        );
    }
}

fn remember_me_requested(uri: &Uri) -> bool {
    let Ok(Query(parameters)) = Query::<HashMap<String, String>>::try_from_uri(uri) else {
        return false;
    };
    parameters.get("remember-me").is_some_and(|value| {
        value.eq_ignore_ascii_case("true")
            || value.eq_ignore_ascii_case("on")
            || value.eq_ignore_ascii_case("yes")
            || value == "1"
    })
}
