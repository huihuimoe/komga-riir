//! The authorization code request and the checks on the response that comes back to the
//! callback (Spring Security `DefaultOAuth2AuthorizationRequestResolver` and
//! `OAuth2LoginAuthenticationFilter`).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::ClientRegistration;

/// How long a started login may take to come back to the callback.
pub const FLOW_TTL: Duration = Duration::from_secs(5 * 60);

/// What the callback needs to check a response and exchange its code. It lives in the browser
/// between the redirect and the callback, so it must be kept confidential and tamper-proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationRequest {
    pub registration_id: String,
    /// The expanded callback URI, sent again with the code.
    pub redirect_uri: String,
    state: String,
    code_verifier: String,
    /// Only for OpenID Connect; the provider receives its SHA-256 hash.
    nonce: Option<String>,
    /// Seconds since the Unix epoch.
    issued_at: u64,
}

/// The query parameters of a callback.
#[derive(Debug, Default, Deserialize)]
pub struct AuthorizationResponse {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Why a callback is rejected; [`Self::error_code`] is what the login page shows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthorizationError {
    /// Neither a code nor an error came with a state.
    #[error("the callback is not an authorization response")]
    InvalidRequest,
    /// No login was started in this browser for this provider, it expired, or the state differs.
    #[error("no matching authorization request")]
    RequestNotFound,
    /// The provider refused, at the callback or the token endpoint, with its own error code.
    #[error("the provider returned {0}")]
    Provider(String),
    /// The token endpoint could not be reached or answered without a bearer access token.
    #[error("invalid token response")]
    InvalidTokenResponse,
    #[error("no user info endpoint is configured")]
    MissingUserInfoUri,
    #[error("no user name attribute is configured")]
    MissingUserNameAttribute,
    /// The user info endpoint failed or answered without the user name attribute.
    #[error("invalid user info response")]
    InvalidUserInfoResponse,
    /// An OpenID Connect provider without a JWK set URI.
    #[error("no JWK set URI is configured")]
    MissingSignatureVerifier,
    /// The ID token is missing, its signature or claims are wrong, or no key verifies it.
    #[error("invalid ID token")]
    InvalidIdToken,
    /// The ID token was not issued for this login.
    #[error("invalid nonce")]
    InvalidNonce,
}

impl AuthorizationError {
    #[must_use]
    pub fn error_code(&self) -> &str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::RequestNotFound => "authorization_request_not_found",
            Self::Provider(code) => code,
            Self::InvalidTokenResponse => "invalid_token_response",
            Self::MissingUserInfoUri => "missing_user_info_uri",
            Self::MissingUserNameAttribute => "missing_user_name_attribute",
            Self::InvalidUserInfoResponse => "invalid_user_info_response",
            Self::MissingSignatureVerifier => "missing_signature_verifier",
            Self::InvalidIdToken => "invalid_id_token",
            Self::InvalidNonce => "invalid_nonce",
        }
    }
}

impl ClientRegistration {
    /// Starts a login: the provider URL to send the browser to, and the request to keep for the
    /// callback. PKCE (S256) is always used; OpenID Connect logins also carry a nonce.
    ///
    /// `base_url` is the server's external `<scheme>://<host>[:<port>][<prefix>]`.
    #[must_use]
    pub fn authorization_request(&self, base_url: &Url) -> (Url, AuthorizationRequest) {
        let request = AuthorizationRequest {
            registration_id: self.registration_id.clone(),
            redirect_uri: expand_redirect_uri(&self.redirect_uri, base_url, &self.registration_id),
            state: random_token(),
            code_verifier: random_token(),
            nonce: self.is_openid().then(random_token),
            issued_at: unix_seconds(SystemTime::now()),
        };
        let mut url = self.provider.authorization_uri.clone();
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &self.client_id);
            if !self.scopes.is_empty() {
                query.append_pair("scope", &self.scopes.join(" "));
            }
            query
                .append_pair("state", &request.state)
                .append_pair("redirect_uri", &request.redirect_uri)
                .append_pair("code_challenge", &sha256(&request.code_verifier))
                .append_pair("code_challenge_method", "S256");
            if let Some(nonce) = &request.nonce {
                query.append_pair("nonce", &sha256(nonce));
            }
        }
        (url, request)
    }
}

impl AuthorizationRequest {
    /// Checks a callback for `registration_id` against the request started in this browser, and
    /// returns that request with the code to exchange. Spring's order applies: a malformed
    /// response first, then the stored request, then the provider's error.
    ///
    /// # Errors
    /// See [`AuthorizationError`].
    pub fn validate<'a>(
        request: Option<&'a Self>,
        registration_id: &str,
        response: &AuthorizationResponse,
        now: SystemTime,
    ) -> Result<(&'a Self, String), AuthorizationError> {
        let Some(state) = response.state.as_deref() else {
            return Err(AuthorizationError::InvalidRequest);
        };
        if response.code.is_none() && response.error.is_none() {
            return Err(AuthorizationError::InvalidRequest);
        }
        let request = request
            .filter(|request| {
                request.registration_id == registration_id
                    && unix_seconds(now) < request.issued_at.saturating_add(FLOW_TTL.as_secs())
                    && constant_time_eq(request.state.as_bytes(), state.as_bytes())
            })
            .ok_or(AuthorizationError::RequestNotFound)?;
        match (&response.error, &response.code) {
            (Some(error), _) => Err(AuthorizationError::Provider(error.clone())),
            (None, code) => Ok((request, code.clone().unwrap_or_default())),
        }
    }

    #[must_use]
    pub(crate) fn code_verifier(&self) -> &str {
        &self.code_verifier
    }

    #[must_use]
    pub(crate) fn nonce(&self) -> Option<&str> {
        self.nonce.as_deref()
    }
}

/// Spring's `{baseUrl}`, `{baseScheme}`, `{baseHost}`, `{basePort}`, `{basePath}`, `{action}`
/// and `{registrationId}` placeholders. `{basePort}` is empty for the scheme's default port.
fn expand_redirect_uri(template: &str, base_url: &Url, registration_id: &str) -> String {
    let base = base_url.as_str().trim_end_matches('/');
    let port = base_url
        .port()
        .map(|port| port.to_string())
        .unwrap_or_default();
    template
        .replace("{baseUrl}", base)
        .replace("{baseScheme}", base_url.scheme())
        .replace("{baseHost}", base_url.host_str().unwrap_or_default())
        .replace("{basePort}", &port)
        .replace("{basePath}", base_url.path().trim_end_matches('/'))
        .replace("{action}", "login")
        .replace("{registrationId}", registration_id)
}

/// 32 random bytes, base64url without padding (PKCE allows 43 to 128 characters).
fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system provides random bytes");
    URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn sha256(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

fn unix_seconds(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (l, r)| difference | (l ^ r))
            == 0
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::RegistrationProperties;
    use crate::builder::common_provider;

    fn google() -> ClientRegistration {
        let mut builder = common_provider("google").expect("Google is a common provider")("google");
        builder.apply_registration(&RegistrationProperties {
            client_id: Some("komga".to_owned()),
            ..RegistrationProperties::default()
        });
        builder.build().expect("Google registration")
    }

    fn base() -> Url {
        Url::parse("https://komga.test:8443/prefix").expect("base URL")
    }

    #[test]
    fn authorization_url_carries_pkce_state_and_hashed_nonce() {
        let (url, request) = google().authorization_request(&base());

        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["response_type"], "code");
        assert_eq!(query["client_id"], "komga");
        assert_eq!(query["scope"], "openid profile email");
        assert_eq!(query["state"], request.state);
        assert_eq!(
            query["redirect_uri"],
            "https://komga.test:8443/prefix/login/oauth2/code/google"
        );
        assert_eq!(query["code_challenge"], sha256(request.code_verifier()));
        assert_eq!(query["code_challenge_method"], "S256");
        let nonce = request
            .nonce()
            .expect("OpenID Connect logins carry a nonce");
        assert_eq!(query["nonce"], sha256(nonce));
        assert_eq!(request.code_verifier().len(), 43);
    }

    #[test]
    fn redirect_uri_placeholders_expand() {
        let expanded = expand_redirect_uri(
            "{baseScheme}://{baseHost}:{basePort}{basePath}/{action}/{registrationId}",
            &base(),
            "idp",
        );
        assert_eq!(expanded, "https://komga.test:8443/prefix/login/idp");
    }

    #[test]
    fn callbacks_are_checked_in_spring_order() {
        let (_, request) = google().authorization_request(&base());
        let now = SystemTime::now();
        let response =
            |code: Option<&str>, state: Option<&str>, error: Option<&str>| AuthorizationResponse {
                code: code.map(str::to_owned),
                state: state.map(str::to_owned),
                error: error.map(str::to_owned),
            };
        let check = |request: Option<&AuthorizationRequest>, id: &str, r, now| {
            AuthorizationRequest::validate(request, id, &r, now).map(|(_, code)| code)
        };
        let state = request.state.as_str();

        assert_eq!(
            check(
                Some(&request),
                "google",
                response(Some("c"), None, None),
                now
            ),
            Err(AuthorizationError::InvalidRequest)
        );
        assert_eq!(
            check(
                Some(&request),
                "google",
                response(None, Some(state), None),
                now
            ),
            Err(AuthorizationError::InvalidRequest)
        );
        assert_eq!(
            check(None, "google", response(Some("c"), Some(state), None), now),
            Err(AuthorizationError::RequestNotFound)
        );
        for (id, state, at) in [
            ("github", state, now),
            ("google", "other", now),
            ("google", state, now + FLOW_TTL),
        ] {
            assert_eq!(
                check(
                    Some(&request),
                    id,
                    response(Some("c"), Some(state), None),
                    at
                ),
                Err(AuthorizationError::RequestNotFound),
                "{id} {state}"
            );
        }
        assert_eq!(
            check(
                Some(&request),
                "google",
                response(None, Some(state), Some("access_denied")),
                now
            )
            .map_err(|error| error.error_code().to_owned()),
            Err("access_denied".to_owned())
        );
        assert_eq!(
            check(
                Some(&request),
                "google",
                response(Some("c"), Some(state), None),
                now
            ),
            Ok("c".to_owned())
        );
    }
}
