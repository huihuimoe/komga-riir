//! Kotlin `KoboProxy`: requests Komga cannot answer, forwarded to the Kobo store.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::http::header::{ACCEPT, ACCEPT_LANGUAGE, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use reqwest::Client;
use reqwest::redirect::Policy;
use serde_json::Value;

use super::SYNC_TOKEN;
use crate::error::ApiError;

/// The only request headers the store gets besides `x-kobo-*`: cookies, Komga credentials and
/// forwarding headers stay here.
const FORWARDED_HEADERS: [HeaderName; 5] = [
    AUTHORIZATION,
    USER_AGENT,
    ACCEPT,
    ACCEPT_LANGUAGE,
    CONTENT_TYPE,
];

#[derive(Clone)]
pub struct KoboStore {
    client: Client,
    url: Arc<str>,
}

/// A store answer, with only its `x-kobo-*` headers.
pub(super) struct StoreReply {
    pub(super) status: StatusCode,
    pub(super) headers: HeaderMap,
    pub(super) body: Option<Value>,
}

#[derive(Debug)]
pub(super) enum StoreError {
    /// The store answered with an error status.
    Status(StatusCode),
    /// The store could not be reached in time, redirected, or did not answer JSON.
    BadGateway(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(status) => write!(formatter, "Kobo store answered {status}"),
            Self::BadGateway(reason) => write!(formatter, "Kobo store failed: {reason}"),
        }
    }
}

impl StoreError {
    pub(super) fn into_api_error(self, path: &str) -> ApiError {
        match self {
            Self::Status(status) => {
                ApiError::new(status, status.canonical_reason().unwrap_or_default(), path)
            }
            Self::BadGateway(_) => ApiError::new(StatusCode::BAD_GATEWAY, "Bad Gateway", path),
        }
    }
}

impl KoboStore {
    /// # Panics
    /// When the TLS backend cannot be set up, which no configuration can fix.
    #[must_use]
    pub fn new(url: String, timeout: Duration) -> Self {
        let client = komga_oauth::http_client(|builder| {
            builder
                .connect_timeout(timeout)
                .read_timeout(timeout)
                // A redirect could carry the device's store credentials to another host.
                .redirect(Policy::none())
        });
        Self {
            client,
            url: url.into(),
        }
    }

    /// Sends the request to the same path under the store, without the `/kobo/{key}` prefix.
    /// `store_token` is the store's own sync token, which replaces the device's Komga one.
    pub(super) async fn forward(
        &self,
        method: Method,
        uri: &Uri,
        headers: &HeaderMap,
        body: Bytes,
        store_token: Option<&str>,
    ) -> Result<StoreReply, StoreError> {
        let mut request = self
            .client
            .request(method, self.store_url(uri))
            .headers(forwarded_headers(headers, store_token));
        if !body.is_empty() {
            request = request.body(body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| StoreError::BadGateway(error.to_string()))?;
        let status = response.status();
        if status.is_client_error() || status.is_server_error() {
            return Err(StoreError::Status(status));
        }
        if !status.is_success() {
            return Err(StoreError::BadGateway(format!("unexpected {status}")));
        }
        let headers = kobo_headers(response.headers());
        let body = response
            .bytes()
            .await
            .map_err(|error| StoreError::BadGateway(error.to_string()))?;
        let body = (!body.is_empty())
            .then(|| serde_json::from_slice(&body))
            .transpose()
            .map_err(|error| StoreError::BadGateway(error.to_string()))?;
        Ok(StoreReply {
            status,
            headers,
            body,
        })
    }

    fn store_url(&self, uri: &Uri) -> String {
        // Like Kotlin's `/kobo/[-\w]*(.*)` search, which also skips the context path.
        let path = uri.path().split_once("/kobo/").map_or("", |(_, path)| path);
        let path = path.find('/').map_or("", |slash| &path[slash..]);
        match uri.query() {
            Some(query) => format!("{}{path}?{query}", self.url),
            None => format!("{}{path}", self.url),
        }
    }
}

impl IntoResponse for StoreReply {
    fn into_response(self) -> Response {
        match self.body {
            Some(body) => (self.status, self.headers, Json(body)).into_response(),
            None => (self.status, self.headers).into_response(),
        }
    }
}

fn is_kobo_header(name: &HeaderName) -> bool {
    name.as_str().starts_with("x-kobo-")
}

fn forwarded_headers(headers: &HeaderMap, store_token: Option<&str>) -> HeaderMap {
    let mut forwarded: HeaderMap = headers
        .iter()
        .filter(|(name, _)| {
            FORWARDED_HEADERS.contains(name) || (is_kobo_header(name) && *name != SYNC_TOKEN)
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    if let Some(token) = store_token.filter(|token| !token.is_empty())
        && let Ok(token) = HeaderValue::from_str(token)
    {
        forwarded.insert(SYNC_TOKEN, token);
    }
    forwarded
}

fn kobo_headers(headers: &HeaderMap) -> HeaderMap {
    headers
        .iter()
        .filter(|(name, _)| is_kobo_header(name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, Uri};

    use super::{KoboStore, forwarded_headers};

    #[test]
    fn store_urls_drop_the_context_and_kobo_prefix_and_keep_the_query() {
        let store = KoboStore::new(
            "https://store".to_owned(),
            std::time::Duration::from_secs(1),
        );
        let url = |uri: &str| store.store_url(&uri.parse::<Uri>().unwrap());
        assert_eq!(
            url("/kobo/key-1/v1/library/sync"),
            "https://store/v1/library/sync"
        );
        assert_eq!(
            url("/kobo/key/v1/x?a=1&b=%20"),
            "https://store/v1/x?a=1&b=%20"
        );
        assert_eq!(url("/komga/kobo/key/v1/x"), "https://store/v1/x");
    }

    #[test]
    fn only_device_headers_reach_the_store() {
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("authorization", "Bearer device"),
            ("cookie", "KOMGA-SESSION=secret"),
            ("x-forwarded-host", "komga.local"),
            ("x-kobo-deviceid", "device"),
            ("x-kobo-synctoken", "KOMGA.token"),
            ("user-agent", "Kobo"),
        ] {
            headers.insert(name, HeaderValue::from_static(value));
        }
        let forwarded = forwarded_headers(&headers, Some("store-token"));
        let mut names: Vec<_> = forwarded.keys().map(|name| name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "authorization",
                "user-agent",
                "x-kobo-deviceid",
                "x-kobo-synctoken"
            ]
        );
        assert_eq!(forwarded["x-kobo-synctoken"], "store-token");
        assert!(!forwarded_headers(&headers, None).contains_key("x-kobo-synctoken"));
    }
}
