//! Session/remember-me cookie helpers and Spring Security unauthorized challenge responses.

use axum::extract::Request;
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use cookie::Cookie;
use komga_service::RememberMeSettings;

use super::{REMEMBER_ME_COOKIE_NAME, SESSION_COOKIE_NAME};
use crate::accept;
use crate::error::ApiError;
use crate::mvc::servlet_uri::{self, RequestContext};
use crate::opds;

/// Spring Session's `DefaultCookieSerializer`: the context path and `/`.
pub(super) fn session_cookie_path(context_path: &str) -> String {
    format!("{context_path}/")
}

pub(super) fn append_session_cookie(
    response: &mut Response,
    token: &str,
    max_age: std::time::Duration,
    secure: bool,
    context_path: &str,
) {
    let cookie = Cookie::build((SESSION_COOKIE_NAME, token.to_owned()))
        .path(session_cookie_path(context_path))
        .http_only(true)
        .secure(secure)
        .same_site(cookie::SameSite::Lax)
        .max_age(
            cookie::time::Duration::try_from(max_age)
                .expect("the session lifetime fits a cookie Max-Age"),
        )
        .build();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie.to_string())
            .expect("session cookie serialization produces a valid header value"),
    );
}

pub(super) fn append_remember_me_cookie(
    response: &mut Response,
    token: String,
    settings: &RememberMeSettings,
    secure: bool,
    context_path: &str,
) {
    let mut builder = Cookie::build((REMEMBER_ME_COOKIE_NAME, token))
        .path(remember_me_cookie_path(context_path))
        .http_only(true)
        .secure(secure);
    if settings.validity_seconds() >= 0 {
        builder = builder.max_age(cookie::time::Duration::seconds(i64::from(
            settings.validity_seconds(),
        )));
    }
    let cookie = builder.build();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie.to_string())
            .expect("remember-me cookie serialization produces a valid header value"),
    );
}

/// `AbstractRememberMeServices.getCookiePath`: the context path, or `/` at the root.
fn remember_me_cookie_path(context_path: &str) -> &str {
    if context_path.is_empty() {
        "/"
    } else {
        context_path
    }
}

pub(super) fn append_expired_remember_me_cookie(
    response: &mut Response,
    secure: bool,
    context_path: &str,
) {
    let cookie = Cookie::build((REMEMBER_ME_COOKIE_NAME, ""))
        .path(remember_me_cookie_path(context_path).to_owned())
        .secure(secure)
        .max_age(cookie::time::Duration::ZERO)
        .build();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie.to_string())
            .expect("expired remember-me cookie serialization produces a valid header value"),
    );
}

/// Kotlin's entry points in Spring Security's order: OPDS 2 paths, then `HttpBasicConfigurer`'s
/// (an empty 401 to scripts, a Basic challenge), then the OPDS 2 one as the default.
pub(super) fn unauthorized(request: &Request) -> Response {
    let headers = request.headers();
    let opds_v2 = request.uri().path().starts_with("/opds/v2/");
    if !opds_v2 && is_xml_http_request(headers) {
        StatusCode::UNAUTHORIZED.into_response()
    } else if !opds_v2 && accept::prefers_basic_challenge(headers) {
        let path = servlet_uri::original_path(request);
        let mut response =
            ApiError::new(StatusCode::UNAUTHORIZED, "Unauthorized", path).into_response();
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static(r#"Basic realm="Realm""#),
        );
        response
    } else {
        opds::authentication_required(&RequestContext::from_extensions(request.extensions()))
    }
}

pub(super) fn is_xml_http_request(headers: &HeaderMap) -> bool {
    headers
        .get("x-requested-with")
        .is_some_and(|value| value == "XMLHttpRequest")
}
