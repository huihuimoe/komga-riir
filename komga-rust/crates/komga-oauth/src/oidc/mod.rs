//! The OpenID Connect part of a login: Spring Security `OidcAuthorizationCodeAuthenticationProvider`
//! and `OidcIdTokenValidator`.

use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::jwk::{Jwk, JwkSet, PublicKeyUse};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde_json::{Map, Value};
use url::Url;

use crate::authorization::sha256;
use crate::{AuthorizationError, AuthorizationRequest, ClientRegistration, ProviderClient};

/// Spring's default allowance for `exp` and `iat`.
const CLOCK_SKEW: Duration = Duration::from_secs(60);
/// Asymmetric algorithms only: a MAC would need the client secret, which no JWK set carries.
const ALGORITHMS: [Algorithm; 9] = [
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::EdDSA,
];

/// The claims of a verified ID token.
pub(crate) type Claims = Map<String, Value>;

impl ClientRegistration {
    /// The claims of the verified ID token of a token response.
    pub(crate) async fn id_token_claims(
        &self,
        client: &ProviderClient,
        request: &AuthorizationRequest,
        id_token: Option<&str>,
    ) -> Result<Claims, AuthorizationError> {
        let id_token = id_token.ok_or(AuthorizationError::InvalidIdToken)?;
        let claims = self.verify_id_token(client, id_token).await?;
        if let Some(nonce) = request.nonce()
            && claims.get("nonce").and_then(Value::as_str) != Some(sha256(nonce).as_str())
        {
            return Err(AuthorizationError::InvalidNonce);
        }
        Ok(claims)
    }

    async fn verify_id_token(
        &self,
        client: &ProviderClient,
        id_token: &str,
    ) -> Result<Claims, AuthorizationError> {
        let uri = self
            .provider
            .jwk_set_uri
            .as_ref()
            .ok_or(AuthorizationError::MissingSignatureVerifier)?;
        let header = jsonwebtoken::decode_header(id_token)
            .map_err(|_| AuthorizationError::InvalidIdToken)?;
        if !ALGORITHMS.contains(&header.alg) {
            return Err(AuthorizationError::InvalidIdToken);
        }
        let kid = header.kid.as_deref();
        let cached = client
            .key_sets
            .cached(uri)
            .filter(|set| candidates(set, kid, header.alg).next().is_some());
        let set = match cached {
            Some(set) => set,
            None => {
                let set = self.fetch_key_set(client, uri).await?;
                client.key_sets.store(uri.clone(), Arc::clone(&set));
                set
            }
        };
        let validation = self.validation(header.alg);
        let claims = candidates(&set, kid, header.alg)
            .filter_map(|jwk| DecodingKey::from_jwk(jwk).ok())
            .find_map(|key| jsonwebtoken::decode::<Claims>(id_token, &key, &validation).ok())
            .ok_or(AuthorizationError::InvalidIdToken)?
            .claims;
        if self.has_valid_claims(&claims) {
            Ok(claims)
        } else {
            Err(AuthorizationError::InvalidIdToken)
        }
    }

    async fn fetch_key_set(
        &self,
        client: &ProviderClient,
        uri: &Url,
    ) -> Result<Arc<JwkSet>, AuthorizationError> {
        let body = self
            .fetch_json(client.http.get(uri.clone()), "JWK set")
            .await
            .ok_or(AuthorizationError::InvalidIdToken)?;
        serde_json::from_value(body)
            .map(Arc::new)
            .map_err(|_| AuthorizationError::InvalidIdToken)
    }

    /// Signature, `iss`, `aud` and `exp`; the rest of `OidcIdTokenValidator` is
    /// [`Self::has_valid_claims`].
    fn validation(&self, algorithm: Algorithm) -> Validation {
        let mut validation = Validation::new(algorithm);
        validation.leeway = CLOCK_SKEW.as_secs();
        validation.set_required_spec_claims(&["iss", "sub", "aud", "exp"]);
        validation.set_audience(&[&self.client_id]);
        if let Some(issuer) = &self.provider.issuer_uri {
            validation.set_issuer(&[issuer]);
        }
        validation
    }

    /// `iat` must be present and not in the future; with several audiences an `azp` must name
    /// this client, and any `azp` must be this client.
    fn has_valid_claims(&self, claims: &Claims) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let issued_in_time = claims
            .get("iat")
            .and_then(Value::as_f64)
            .is_some_and(|iat| iat <= (now + CLOCK_SKEW).as_secs_f64());
        let several_audiences = claims
            .get("aud")
            .and_then(Value::as_array)
            .is_some_and(|audiences| audiences.len() > 1);
        let valid_party = match claims.get("azp") {
            Some(Value::String(party)) => *party == self.client_id,
            Some(_) => false,
            None => !several_audiences,
        };
        issued_in_time && valid_party
    }
}

/// The signing keys that may have signed a token with this header.
fn candidates<'a>(
    set: &'a JwkSet,
    kid: Option<&'a str>,
    algorithm: Algorithm,
) -> impl Iterator<Item = &'a Jwk> {
    set.keys.iter().filter(move |jwk| {
        let common = &jwk.common;
        kid.is_none_or(|kid| common.key_id.as_deref() == Some(kid))
            && !matches!(common.public_key_use, Some(PublicKeyUse::Encryption))
            && common.key_algorithm.is_none_or(|key_algorithm| {
                Algorithm::from_str(&key_algorithm.to_string()).ok() == Some(algorithm)
            })
    })
}

#[cfg(test)]
mod tests;
