//! Mapping of the client properties to registrations (`OAuth2ClientPropertiesMapper`).

use reqwest::Client;

use crate::builder::{self, Builder};
use crate::client::ProviderClient;
use crate::discovery;
use crate::properties::{OAuth2ClientProperties, RegistrationError, RegistrationProperties};
use crate::registration::ClientRegistration;

impl OAuth2ClientProperties {
    /// Builds every registration, fetching the metadata of the providers that name an issuer.
    ///
    /// # Errors
    /// Fails on the first registration that is incomplete, unsupported, or whose issuer cannot be
    /// resolved.
    pub async fn resolve(
        &self,
        client: &ProviderClient,
    ) -> Result<Vec<ClientRegistration>, RegistrationError> {
        let mut registrations = Vec::with_capacity(self.registration.len());
        for (id, properties) in &self.registration {
            if properties
                .client_id
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(RegistrationError::MissingClientId(id.clone()));
            }
            let mut builder = self.provider_builder(id, properties, &client.http).await?;
            builder.apply_registration(properties);
            registrations.push(builder.build()?);
        }
        Ok(registrations)
    }

    async fn provider_builder(
        &self,
        id: &str,
        properties: &RegistrationProperties,
        client: &Client,
    ) -> Result<Builder, RegistrationError> {
        let provider_id = properties.provider.as_deref().unwrap_or(id);
        let provider = self.provider.get(provider_id);
        if let Some(provider) = provider
            && let Some(issuer) = &provider.issuer_uri
        {
            let metadata = discovery::discover(client, issuer)
                .await
                .map_err(|source| RegistrationError::Discovery {
                    registration: id.to_owned(),
                    source,
                })?;
            let mut builder = Builder::from_metadata(id, issuer, metadata);
            builder.apply_provider(provider);
            return Ok(builder);
        }
        let mut builder = match (builder::common_provider(provider_id), provider) {
            (Some(common), _) => common(id),
            (None, Some(_)) => Builder::new(id),
            (None, None) => {
                return Err(match &properties.provider {
                    Some(provider) => RegistrationError::UnknownProvider {
                        registration: id.to_owned(),
                        provider: provider.clone(),
                    },
                    None => RegistrationError::MissingProvider(id.to_owned()),
                });
            }
        };
        if let Some(provider) = provider {
            builder.apply_provider(provider);
        }
        Ok(builder)
    }
}

#[cfg(test)]
mod tests {
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::*;
    use crate::builder::{AUTHORIZATION_CODE, DEFAULT_REDIRECT_URI, SUB};
    use crate::properties::ProviderProperties;
    use crate::registration::{ClientAuthenticationMethod, ClientSecret};

    fn registration(client_id: &str) -> RegistrationProperties {
        RegistrationProperties {
            client_id: Some(client_id.to_owned()),
            ..RegistrationProperties::default()
        }
    }

    fn properties(
        registrations: impl IntoIterator<Item = (&'static str, RegistrationProperties)>,
        providers: impl IntoIterator<Item = (&'static str, ProviderProperties)>,
    ) -> OAuth2ClientProperties {
        OAuth2ClientProperties {
            registration: registrations
                .into_iter()
                .map(|(id, r)| (id.to_owned(), r))
                .collect(),
            provider: providers
                .into_iter()
                .map(|(id, p)| (id.to_owned(), p))
                .collect(),
        }
    }

    async fn resolve_one(
        properties: &OAuth2ClientProperties,
    ) -> Result<ClientRegistration, RegistrationError> {
        let mut found = properties.resolve(&ProviderClient::new()).await?;
        assert_eq!(found.len(), 1);
        Ok(found.remove(0))
    }

    #[tokio::test]
    async fn common_providers_supply_defaults_the_registration_overrides() {
        let github = RegistrationProperties {
            client_secret: Some(ClientSecret::new("secret".to_owned())),
            scope: Some(vec!["user:email".to_owned()]),
            ..registration("client")
        };
        let found = resolve_one(&properties([("GitHub", github)], []))
            .await
            .unwrap();

        assert_eq!(found.registration_id, "GitHub");
        assert_eq!(found.client_name, "GitHub");
        assert_eq!(found.redirect_uri, DEFAULT_REDIRECT_URI);
        assert_eq!(found.scopes, ["user:email"]);
        assert_eq!(
            found.client_authentication_method,
            ClientAuthenticationMethod::ClientSecretBasic
        );
        assert_eq!(
            found.provider.token_uri.as_str(),
            "https://github.com/login/oauth/access_token"
        );
        assert!(!found.is_openid());
        assert_eq!(format!("{:?}", found.client_secret), "***");
    }

    #[tokio::test]
    async fn custom_providers_need_every_endpoint_and_a_grant_type() {
        let provider = ProviderProperties {
            authorization_uri: Some("https://idp.test/auth".to_owned()),
            token_uri: Some("https://idp.test/token".to_owned()),
            ..ProviderProperties::default()
        };
        let without_grant = properties(
            [("idp", registration("client"))],
            [("idp", provider.clone())],
        );
        let error = resolve_one(&without_grant).await.unwrap_err();
        assert!(matches!(
            error,
            RegistrationError::Missing {
                field: "authorizationGrantType",
                ..
            }
        ));

        let complete = RegistrationProperties {
            authorization_grant_type: Some(AUTHORIZATION_CODE.to_owned()),
            redirect_uri: Some(DEFAULT_REDIRECT_URI.to_owned()),
            scope: Some(vec!["openid".to_owned(), "email".to_owned()]),
            ..registration("client")
        };
        let found = resolve_one(&properties([("idp", complete)], [("idp", provider)]))
            .await
            .unwrap();
        assert_eq!(found.client_name, "idp");
        assert!(found.is_openid());
        // Spring deduces a public client from the missing secret.
        assert_eq!(
            found.client_authentication_method,
            ClientAuthenticationMethod::None
        );
    }

    #[tokio::test]
    async fn registrations_must_name_a_known_provider() {
        let missing = resolve_one(&properties([("idp", registration("client"))], [])).await;
        assert!(matches!(missing, Err(RegistrationError::MissingProvider(id)) if id == "idp"));

        let unknown = RegistrationProperties {
            provider: Some("nowhere".to_owned()),
            ..registration("client")
        };
        let unknown = resolve_one(&properties([("idp", unknown)], [])).await;
        assert!(
            matches!(unknown, Err(RegistrationError::UnknownProvider { provider, .. }) if provider == "nowhere")
        );

        let blank = resolve_one(&properties([("google", registration(" "))], [])).await;
        assert!(matches!(blank, Err(RegistrationError::MissingClientId(_))));
    }

    #[tokio::test]
    async fn unusable_flows_and_values_are_rejected() {
        let cases = [
            RegistrationProperties {
                authorization_grant_type: Some("client_credentials".to_owned()),
                ..registration("client")
            },
            RegistrationProperties {
                client_authentication_method: Some("private_key_jwt".to_owned()),
                ..registration("client")
            },
        ];
        for case in cases {
            let error = resolve_one(&properties([("google", case)], []))
                .await
                .unwrap_err();
            assert!(
                matches!(error, RegistrationError::Unsupported { .. }),
                "{error}"
            );
        }
        let bad_scope = RegistrationProperties {
            scope: Some(vec!["a\"b".to_owned()]),
            ..registration("client")
        };
        let error = resolve_one(&properties([("google", bad_scope)], []))
            .await
            .unwrap_err();
        assert!(
            matches!(error, RegistrationError::InvalidScope { .. }),
            "{error}"
        );

        let bad_url = ProviderProperties {
            token_uri: Some("not a url".to_owned()),
            ..ProviderProperties::default()
        };
        let error = resolve_one(&properties(
            [("google", registration("client"))],
            [("google", bad_url)],
        ))
        .await
        .unwrap_err();
        assert!(
            matches!(
                error,
                RegistrationError::InvalidUrl {
                    field: "tokenUri",
                    ..
                }
            ),
            "{error}"
        );
    }

    #[tokio::test]
    async fn issuers_supply_the_discovered_endpoints() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let metadata = json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/auth"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/certs"),
            "userinfo_endpoint": format!("{issuer}/userinfo"),
            "token_endpoint_auth_methods_supported": ["client_secret_post"],
        });
        let router = Router::new().route(
            "/.well-known/openid-configuration",
            get(move || async move { Json(metadata) }),
        );
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let provider = ProviderProperties {
            issuer_uri: Some(issuer.clone()),
            user_info_uri: Some(format!("{issuer}/me")),
            ..ProviderProperties::default()
        };
        // An issuer wins over the common provider of the same name.
        let found = resolve_one(&properties(
            [("google", registration("client"))],
            [("google", provider)],
        ))
        .await
        .unwrap();

        assert_eq!(found.client_name, issuer);
        assert_eq!(found.scopes, Vec::<String>::new());
        assert_eq!(
            found.client_authentication_method,
            ClientAuthenticationMethod::ClientSecretPost
        );
        assert_eq!(
            found.provider.authorization_uri.as_str(),
            format!("{issuer}/auth")
        );
        assert_eq!(
            found.provider.user_info_uri.unwrap().as_str(),
            format!("{issuer}/me")
        );
        assert_eq!(found.provider.issuer_uri.as_deref(), Some(issuer.as_str()));
        assert_eq!(found.provider.user_name_attribute.as_deref(), Some(SUB));
    }
}
