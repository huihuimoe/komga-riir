//! Shared HTTP client and JWK cache for talking to OAuth 2.0 / OIDC providers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use reqwest::header::ACCEPT;
use reqwest::redirect::Policy;
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use tracing::warn;
use url::Url;

use crate::outbound;
use crate::registration::ClientRegistration;

/// How long a fetched JWK set is trusted before it is fetched again; a token signed with a key
/// the cached set lacks fetches it at once, so rotated keys are picked up without waiting.
const KEY_SET_TTL: Duration = Duration::from_secs(5 * 60);

/// The HTTP client and the cached JWK sets that every login shares.
#[derive(Clone)]
pub struct ProviderClient {
    pub(crate) http: Client,
    pub(crate) key_sets: KeySets,
}

impl ProviderClient {
    /// # Panics
    /// When the TLS backend cannot be set up, which no configuration can fix.
    #[must_use]
    pub fn new() -> Self {
        Self {
            http: http_client(),
            key_sets: KeySets::default(),
        }
    }
}

impl Default for ProviderClient {
    fn default() -> Self {
        Self::new()
    }
}

/// The client for every request to a provider: discovery, token, JWK set, and user info.
fn http_client() -> Client {
    outbound::http_client(|builder| {
        builder
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            // Endpoints are configured or discovered; a redirect could carry codes or tokens away.
            .redirect(Policy::none())
    })
}

type FetchedSet = (Instant, Arc<JwkSet>);

/// The JWK sets fetched so far, by URI.
#[derive(Clone, Default)]
pub(crate) struct KeySets(Arc<Mutex<HashMap<Url, FetchedSet>>>);

impl KeySets {
    pub(crate) fn cached(&self, uri: &Url) -> Option<Arc<JwkSet>> {
        let sets = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        sets.get(uri)
            .filter(|(fetched, _)| fetched.elapsed() < KEY_SET_TTL)
            .map(|(_, set)| Arc::clone(set))
    }

    pub(crate) fn store(&self, uri: Url, set: Arc<JwkSet>) {
        let mut sets = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        sets.insert(uri, (Instant::now(), set));
    }
}

impl ClientRegistration {
    /// The JSON body of a provider response, or none when the request fails or the body is not
    /// JSON. Error statuses still return their body, which may carry an OAuth error code.
    pub(crate) async fn fetch_json(&self, http: RequestBuilder, endpoint: &str) -> Option<Value> {
        let registration = &self.registration_id;
        let response = http
            .header(ACCEPT, "application/json")
            .send()
            .await
            .inspect_err(|error| warn!(registration, %error, "OAuth2 {endpoint} request failed"))
            .ok()?;
        let status = response.status();
        let body = response
            .json::<Value>()
            .await
            .inspect_err(|error| {
                warn!(registration, %status, %error, "OAuth2 {endpoint} response is not JSON");
            })
            .ok()?;
        if status.is_success() || body.get("error").is_some() {
            Some(body)
        } else {
            warn!(registration, %status, "OAuth2 {endpoint} request failed");
            None
        }
    }
}
