//! Spring Security `oauth2Login`: the redirect to the provider and its callback. The request
//! that the callback checks travels in a short-lived encrypted cookie instead of the session.

use std::time::{Duration, Instant};

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use axum::Router;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cookie::{Cookie, SameSite};
use komga_oauth::{AuthorizationError, AuthorizationRequest, AuthorizationResponse, FLOW_TTL};
use komga_service::{ExternalIdentityService, FlowKeyRing, FlowSecret, SignInError};
use komga_types::RecordActivityParams;
use url::Url;
use url::form_urlencoded::byte_serialize;

use super::request_metadata::AuthActivityMetadata;
use super::resolver::{extract_cookie_value, record_activity, success_activity};
use super::{append_session_cookie, session_cookie_path};
use crate::error::ApiError;
use crate::mvc::servlet_uri::RequestContext;
use crate::routes::ApiState;

const FLOW_COOKIE_NAME: &str = "KOMGA-OAUTH2-FLOW";

pub(super) fn router(state: &ApiState) -> Router {
    Router::new()
        .route("/oauth2/authorization/{registration_id}", get(authorize))
        .route("/login/oauth2/code/{registration_id}", get(callback))
        .with_state(state.clone())
}

/// `OAuth2AuthorizationRequestRedirectFilter`. An unknown provider is a 404, where Spring
/// answers 500.
async fn authorize(
    State(state): State<ApiState>,
    Path(registration_id): Path<String>,
    context: RequestContext,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let external = &state.external_identity;
    let Some(registration) = external.registration(&registration_id) else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "Not Found",
            uri.path(),
        ));
    };
    let base_url = base_url(&context, &uri)?;
    let (location, request) = registration.authorization_request(&base_url);
    let keys = flow_keys(external, &uri).await?;
    let secure = base_url.scheme() == "https";
    let value = serde_json::to_string(&request).expect("an authorization request serializes");
    let sealed = seal(&value, &keys.current);
    let cookie = flow_cookie(sealed, secure, context.path(), FLOW_TTL);
    let mut response = redirect(location.as_str());
    append_cookie(&mut response, &cookie.to_string());
    Ok(response)
}

/// `OAuth2LoginAuthenticationFilter`: a signed-in user gets a session and goes to the web UI,
/// failures go back to the login page with the error code, and the flow cookie is cleared
/// whatever the outcome.
async fn callback(
    State(state): State<ApiState>,
    Path(registration_id): Path<String>,
    Query(response): Query<AuthorizationResponse>,
    headers: HeaderMap,
    context: RequestContext,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let external = &state.external_identity;
    let Some(registration) = external.registration(&registration_id) else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "Not Found",
            uri.path(),
        ));
    };
    let secure = base_url(&context, &uri)?.scheme() == "https";
    let keys = flow_keys(external, &uri).await?;
    let request = extract_cookie_value(&headers, FLOW_COOKIE_NAME).and_then(|value| {
        [Some(keys.current), keys.previous]
            .into_iter()
            .flatten()
            .find_map(|secret| open(&value, &secret))
    });
    let loaded_at = Instant::now();
    let outcome = external
        .complete(registration, request.as_ref(), &response)
        .await;
    let source = format!("OAuth2:{}", registration.client_name);
    let metadata = AuthActivityMetadata::from_headers(&headers);
    let mut reply = match outcome {
        Ok(user) => {
            record_activity(&state.auth, &metadata, success_activity(&user, &source)).await;
            match state.auth.sessions().issue_token(&user, loaded_at) {
                Some(token) => {
                    let mut reply = redirect(&format!("{}/?server_redirect=Y", context.path()));
                    let ttl = state.auth.sessions().ttl();
                    append_session_cookie(&mut reply, &token, ttl, secure, context.path());
                    reply
                }
                // The account changed while signing in: sign in again with its new rights.
                None => redirect(&format!("{}/login?server_redirect=Y", context.path())),
            }
        }
        Err(error) => {
            let Some(code) = error.error_code() else {
                return Err(ApiError::internal_error(uri.path(), error));
            };
            // Spring reports only the failures of the provider exchange, not stray callbacks.
            if !matches!(
                error,
                SignInError::Protocol(
                    AuthorizationError::InvalidRequest | AuthorizationError::RequestNotFound
                )
            ) {
                let failure = RecordActivityParams {
                    source: &source,
                    error: Some(code),
                    ..RecordActivityParams::default()
                };
                record_activity(&state.auth, &metadata, failure).await;
            }
            let code: String = byte_serialize(code.as_bytes()).collect();
            redirect(&format!(
                "{}/login?server_redirect=Y&error={code}",
                context.path()
            ))
        }
    };
    let removal = flow_cookie(String::new(), secure, context.path(), Duration::ZERO);
    append_cookie(&mut reply, &removal.to_string());
    Ok(reply)
}

async fn flow_keys(external: &ExternalIdentityService, uri: &Uri) -> Result<FlowKeyRing, ApiError> {
    external
        .flow_keys()
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))
}

/// The external URL of the server, which the callback URI is built from.
fn base_url(context: &RequestContext, uri: &Uri) -> Result<Url, ApiError> {
    Url::parse(&context.url())
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "Bad Request", uri.path()))
}

/// AES-256-GCM with the cookie name as associated data, encoded as base64 of nonce, ciphertext
/// and tag: the cookie crate's private cookie format.
fn seal(value: &str, secret: &FlowSecret) -> String {
    let mut nonce = [0; NONCE_LEN];
    getrandom::fill(&mut nonce).expect("Failed to get random bytes");
    let mut sealed = value.as_bytes().to_vec();
    flow_cipher(secret)
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(FLOW_COOKIE_NAME),
            &mut sealed,
        )
        .expect("a flow request is far below the AES-GCM message limit");
    STANDARD.encode([&nonce[..], &sealed].concat())
}

fn open(value: &str, secret: &FlowSecret) -> Option<AuthorizationRequest> {
    let mut sealed = STANDARD.decode(value).ok()?;
    let (nonce, ciphertext) = sealed.split_at_mut_checked(NONCE_LEN)?;
    let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;
    let value = flow_cipher(secret)
        .open_in_place(nonce, Aad::from(FLOW_COOKIE_NAME), ciphertext)
        .ok()?;
    serde_json::from_slice(value).ok()
}

/// The second half of the secret, which the cookie crate's `Key` encrypts with.
fn flow_cipher(secret: &FlowSecret) -> LessSafeKey {
    let key = UnboundKey::new(&AES_256_GCM, &secret[32..]).expect("32 bytes is an AES-256 key");
    LessSafeKey::new(key)
}

/// SameSite=Lax still sends the cookie on the provider's top-level redirect back.
fn flow_cookie(
    value: String,
    secure: bool,
    context_path: &str,
    max_age: Duration,
) -> Cookie<'static> {
    Cookie::build((FLOW_COOKIE_NAME, value))
        .path(session_cookie_path(context_path))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .max_age(
            max_age
                .try_into()
                .expect("the flow lifetime fits a cookie Max-Age"),
        )
        .build()
}

fn redirect(location: &str) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(location).expect("a URL is a valid header value"),
    );
    response
}

fn append_cookie(response: &mut Response, cookie: &str) {
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(cookie).expect("a serialized cookie is a valid header value"),
    );
}
