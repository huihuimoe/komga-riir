//! Request credential extraction and user resolution.

use axum::http::{HeaderMap, header};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cookie::Cookie;
use komga_types::{ApiKey, KomgaUser, RecordActivityParams};

use super::request_metadata::AuthActivityMetadata;
use super::{AuthState, REMEMBER_ME_COOKIE_NAME, SESSION_COOKIE_NAME};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AuthSource {
    ApiKey,
    Session,
    RememberMe,
    Basic,
}

pub(super) struct ResolvedAuth {
    pub user: KomgaUser,
    pub source: AuthSource,
    pub session_token: Option<String>,
    /// The key an API-key request authenticated with.
    pub api_key: Option<ApiKey>,
}

pub(super) enum RequestAuthResolution {
    Authenticated(Box<ResolvedAuth>),
    /// No usable credentials; an unknown session only counts as absent.
    Anonymous,
    /// Basic or API-key credentials were sent but failed.
    Rejected,
    RejectedRememberMe,
}

/// Resolve a user in the baseline order: session, Basic, API key, then remember-me.
pub(super) async fn resolve_request_auth(
    state: &AuthState,
    headers: &HeaderMap,
) -> Result<RequestAuthResolution, String> {
    let session_auth = extract_session_token(headers).and_then(|token| {
        state
            .sessions()
            .resolve_user(&token)
            .map(|user| (user, token))
    });
    if let Some((user, token)) = session_auth {
        return Ok(authenticated(user, AuthSource::Session, Some(token)));
    }

    let metadata = AuthActivityMetadata::from_headers(headers);
    let Ok(basic_credentials) = extract_basic_credentials(headers) else {
        return Ok(RequestAuthResolution::Rejected);
    };
    if let Some((email, password)) = basic_credentials {
        return resolve_basic(state, &metadata, &email, &password).await;
    }
    if let Some(api_key) = headers.get("X-API-Key") {
        let api_key = api_key.to_str().unwrap_or_default().trim();
        return resolve_api_key(state, &metadata, api_key).await;
    }
    if has_remember_me_cookie(headers) {
        return resolve_remember_me(state, &metadata, headers).await;
    }
    Ok(RequestAuthResolution::Anonymous)
}

async fn resolve_basic(
    state: &AuthState,
    metadata: &AuthActivityMetadata,
    email: &str,
    password: &str,
) -> Result<RequestAuthResolution, String> {
    let user = state
        .identity()
        .authenticate(email, password)
        .await
        .map_err(|error| error.to_string())?;
    let Some(user) = user else {
        let known_user = state
            .identity()
            .find_by_email(email)
            .await
            .map_err(|error| error.to_string())?;
        let params = RecordActivityParams {
            user_id: known_user.as_ref().map(|user| user.id.as_str()),
            email: Some(email),
            ..failure_activity("Password")
        };
        record_activity(state, metadata, params).await;
        return Ok(RequestAuthResolution::Rejected);
    };
    record_activity(state, metadata, success_activity(&user, "Password")).await;
    Ok(authenticated(user, AuthSource::Basic, None))
}

pub(super) async fn resolve_api_key(
    state: &AuthState,
    metadata: &AuthActivityMetadata,
    api_key: &str,
) -> Result<RequestAuthResolution, String> {
    let authenticated_key = state
        .identity()
        .authenticate_api_key(api_key)
        .await
        .map_err(|error| error.to_string())?;
    let Some((user, key)) = authenticated_key else {
        record_activity(state, metadata, failure_activity("ApiKey")).await;
        return Ok(RequestAuthResolution::Rejected);
    };
    let params = RecordActivityParams {
        api_key_id: Some(key.id.as_str()),
        api_key_comment: Some(&key.comment),
        ..success_activity(&user, "ApiKey")
    };
    record_activity(state, metadata, params).await;
    Ok(RequestAuthResolution::Authenticated(Box::new(
        ResolvedAuth {
            user,
            source: AuthSource::ApiKey,
            session_token: None,
            api_key: Some(key),
        },
    )))
}

async fn resolve_remember_me(
    state: &AuthState,
    metadata: &AuthActivityMetadata,
    headers: &HeaderMap,
) -> Result<RequestAuthResolution, String> {
    let Some(token) = extract_remember_me_cookie(headers) else {
        return Ok(RequestAuthResolution::RejectedRememberMe);
    };
    let user = state
        .identity()
        .authenticate_remember_me(&token, state.remember_me_settings())
        .await
        .map_err(|error| error.to_string())?;
    let Some(user) = user else {
        return Ok(RequestAuthResolution::RejectedRememberMe);
    };
    record_activity(state, metadata, success_activity(&user, "RememberMe")).await;
    Ok(authenticated(user, AuthSource::RememberMe, None))
}

fn authenticated(
    user: KomgaUser,
    source: AuthSource,
    session_token: Option<String>,
) -> RequestAuthResolution {
    RequestAuthResolution::Authenticated(Box::new(ResolvedAuth {
        user,
        source,
        session_token,
        api_key: None,
    }))
}

pub(super) fn success_activity<'a>(
    user: &'a KomgaUser,
    source: &'a str,
) -> RecordActivityParams<'a> {
    RecordActivityParams {
        user_id: Some(user.id.as_str()),
        email: Some(&user.email),
        source,
        success: true,
        ..RecordActivityParams::default()
    }
}

fn failure_activity(source: &str) -> RecordActivityParams<'_> {
    RecordActivityParams {
        source,
        error: Some("Bad credentials"),
        ..RecordActivityParams::default()
    }
}

pub(super) async fn record_activity(
    state: &AuthState,
    metadata: &AuthActivityMetadata,
    mut params: RecordActivityParams<'_>,
) {
    params.ip = metadata.ip.as_deref();
    params.user_agent = metadata.user_agent.as_deref();
    state.identity().record_activity(params).await;
}

pub(super) fn extract_session_token(headers: &HeaderMap) -> Option<String> {
    if headers.contains_key("X-Auth-Token") {
        return headers
            .get("X-Auth-Token")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
    }

    extract_cookie_value(headers, SESSION_COOKIE_NAME)
}

pub(super) fn extract_cookie_value(headers: &HeaderMap, target_name: &str) -> Option<String> {
    for cookie_header in headers.get_all(header::COOKIE) {
        let header_str = cookie_header.to_str().ok()?;
        for raw_cookie in Cookie::split_parse(header_str).flatten() {
            if raw_cookie.name() == target_name {
                let value = raw_cookie.value().trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn extract_remember_me_cookie(headers: &HeaderMap) -> Option<String> {
    extract_cookie_value(headers, REMEMBER_ME_COOKIE_NAME)
}

fn has_remember_me_cookie(headers: &HeaderMap) -> bool {
    headers.get_all(header::COOKIE).iter().any(|cookie_header| {
        cookie_header.to_str().is_ok_and(|header_value| {
            Cookie::split_parse(header_value)
                .flatten()
                .any(|cookie| cookie.name() == REMEMBER_ME_COOKIE_NAME)
        })
    })
}

fn extract_basic_credentials(headers: &HeaderMap) -> Result<Option<(String, String)>, ()> {
    let Some(auth_header) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let auth_header = auth_header.to_str().map_err(|_| ())?.trim();
    if !auth_header
        .get(..5)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("Basic"))
    {
        return Ok(None);
    }

    let encoded_credentials = auth_header.get(5..).ok_or(())?.trim();
    if encoded_credentials.is_empty() {
        return Err(());
    }
    let bytes = STANDARD.decode(encoded_credentials).map_err(|_| ())?;
    let decoded = String::from_utf8(bytes).map_err(|_| ())?;
    let (email, password) = decoded.split_once(':').ok_or(())?;
    Ok(Some((email.to_string(), password.to_string())))
}
