//! Provider metadata discovery from an issuer (Spring `ClientRegistrations.fromIssuerLocation`).

use reqwest::Client;
use serde::Deserialize;
use url::Url;

const OIDC_METADATA_PATH: &str = "/.well-known/openid-configuration";
const OAUTH_METADATA_PATH: &str = "/.well-known/oauth-authorization-server";

/// The parts of the provider metadata a registration takes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct ProviderMetadata {
    pub issuer: String,
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: String,
    pub jwks_uri: Option<String>,
    pub userinfo_endpoint: Option<String>,
    pub token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

/// Why an issuer could not be resolved.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("the issuer \"{0}\" is not a valid URL")]
    InvalidIssuer(String),
    #[error("unable to resolve the configuration of the issuer \"{0}\"")]
    NotFound(String),
    #[error("could not read the configuration of the issuer \"{issuer}\" from {url}")]
    Request {
        issuer: String,
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("the configuration at {url} names the issuer \"{found}\" instead of \"{issuer}\"")]
    IssuerMismatch {
        issuer: String,
        url: String,
        found: String,
    },
    #[error("the OpenID configuration at {0} has no jwks_uri")]
    MissingJwksUri(String),
}

/// Fetches the metadata of `issuer`, trying the OpenID Connect location, its RFC 8414 form, and
/// the OAuth 2.0 authorization server location in turn. A 4xx answer moves on to the next one;
/// any other failure stops.
pub(crate) async fn discover(
    client: &Client,
    issuer: &str,
) -> Result<ProviderMetadata, DiscoveryError> {
    let issuer_url = Url::parse(issuer)
        .ok()
        .filter(|url| url.has_host())
        .ok_or_else(|| DiscoveryError::InvalidIssuer(issuer.to_owned()))?;
    let path = issuer_url.path().trim_end_matches('/');
    let locations = [
        (format!("{path}{OIDC_METADATA_PATH}"), Metadata::OpenId),
        (format!("{OIDC_METADATA_PATH}{path}"), Metadata::OpenId),
        (format!("{OAUTH_METADATA_PATH}{path}"), Metadata::OAuth),
    ];
    for (location, kind) in locations {
        let mut url = issuer_url.clone();
        url.set_path(&location);
        url.set_query(None);
        url.set_fragment(None);
        let response = client
            .get(url.clone())
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        let response = match response {
            Ok(response) => response,
            Err(error)
                if error
                    .status()
                    .is_some_and(|status| status.is_client_error()) =>
            {
                continue;
            }
            Err(source) => return Err(request_error(issuer, url, source)),
        };
        let metadata: ProviderMetadata = response
            .json()
            .await
            .map_err(|source| request_error(issuer, url.clone(), source))?;
        if metadata.issuer != issuer {
            return Err(DiscoveryError::IssuerMismatch {
                issuer: issuer.to_owned(),
                url: url.into(),
                found: metadata.issuer,
            });
        }
        if kind == Metadata::OpenId && metadata.jwks_uri.is_none() {
            return Err(DiscoveryError::MissingJwksUri(url.into()));
        }
        return Ok(metadata);
    }
    Err(DiscoveryError::NotFound(issuer.to_owned()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Metadata {
    OpenId,
    OAuth,
}

fn request_error(issuer: &str, url: Url, source: reqwest::Error) -> DiscoveryError {
    DiscoveryError::Request {
        issuer: issuer.to_owned(),
        url: url.into(),
        source,
    }
}

/// Spring's choice among the advertised token endpoint methods; `None` lets the registration
/// deduce one from its secret.
pub(crate) fn token_endpoint_method(
    supported: Option<&[String]>,
) -> Option<crate::ClientAuthenticationMethod> {
    use crate::ClientAuthenticationMethod;

    let Some(supported) = supported else {
        return Some(ClientAuthenticationMethod::ClientSecretBasic);
    };
    let has = |method: &str| supported.iter().any(|value| value == method);
    if has("client_secret_basic") {
        Some(ClientAuthenticationMethod::ClientSecretBasic)
    } else if has("client_secret_post") {
        Some(ClientAuthenticationMethod::ClientSecretPost)
    } else if has("none") {
        Some(ClientAuthenticationMethod::None)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode as AxumStatus;
    use axum::routing::get;
    use axum::{Json, Router};
    use reqwest::StatusCode;
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::*;

    /// Serves the router `build` makes from the server's base URL, and returns that URL.
    async fn serve(build: impl FnOnce(String) -> Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = build(base.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        base
    }

    fn metadata(issuer: &str) -> serde_json::Value {
        json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/auth"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/certs"),
        })
    }

    #[tokio::test]
    async fn issuers_resolve_from_the_first_location_that_answers() {
        let base = serve(|base| {
            let issuer = format!("{base}/tenant");
            Router::new()
                .route(
                    "/tenant/.well-known/openid-configuration",
                    get(|| async { AxumStatus::NOT_FOUND }),
                )
                .route(
                    "/.well-known/oauth-authorization-server/tenant",
                    get(move || async move {
                        Json(json!({"issuer": issuer, "token_endpoint": format!("{issuer}/token")}))
                    }),
                )
        })
        .await;
        let issuer = format!("{base}/tenant");

        let found = discover(&Client::new(), &issuer).await.unwrap();

        assert_eq!(found.token_endpoint, format!("{issuer}/token"));
        assert_eq!(found.jwks_uri, None);
    }

    #[tokio::test]
    async fn metadata_must_name_the_configured_issuer() {
        let base = serve(|_| {
            Router::new().route(
                "/.well-known/openid-configuration",
                get(|| async { Json(metadata("https://elsewhere")) }),
            )
        })
        .await;
        let error = discover(&Client::new(), &base).await.unwrap_err();
        assert!(
            matches!(error, DiscoveryError::IssuerMismatch { .. }),
            "{error}"
        );
    }

    #[tokio::test]
    async fn openid_metadata_needs_a_key_set() {
        let base = serve(|base| {
            let mut without_keys = metadata(&base);
            without_keys.as_object_mut().unwrap().remove("jwks_uri");
            Router::new().route(
                "/.well-known/openid-configuration",
                get(move || async move { Json(without_keys) }),
            )
        })
        .await;
        let error = discover(&Client::new(), &base).await.unwrap_err();
        assert!(
            matches!(error, DiscoveryError::MissingJwksUri(_)),
            "{error}"
        );
    }

    #[tokio::test]
    async fn server_errors_stop_discovery() {
        let base = serve(|_| Router::new().fallback(|| async { AxumStatus::BAD_GATEWAY })).await;
        let error = discover(&Client::new(), &base).await.unwrap_err();
        let DiscoveryError::Request { source, .. } = error else {
            panic!("expected a request error, got {error}");
        };
        assert_eq!(source.status(), Some(StatusCode::BAD_GATEWAY));
    }

    #[test]
    fn token_endpoint_methods_prefer_basic() {
        let methods = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            token_endpoint_method(None),
            Some(crate::ClientAuthenticationMethod::ClientSecretBasic)
        );
        assert_eq!(
            token_endpoint_method(Some(&methods(&["client_secret_post", "none"]))),
            Some(crate::ClientAuthenticationMethod::ClientSecretPost)
        );
        assert_eq!(
            token_endpoint_method(Some(&methods(&["private_key_jwt"]))),
            None
        );
    }
}
