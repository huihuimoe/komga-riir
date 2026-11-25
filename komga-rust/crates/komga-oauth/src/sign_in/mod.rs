//! The code exchange and the user it signs in: Spring Security
//! `RestClientAuthorizationCodeTokenResponseClient`, `DefaultOAuth2UserService` and `OidcUserService`,
//! with Komga's `GithubOAuth2UserService` email lookup. [`crate::oidc`] verifies the ID token.

use serde::Deserialize;
use serde_json::{Map, Value};
use tracing::warn;
use url::form_urlencoded::byte_serialize;

use crate::oidc::Claims;
use crate::{
    AuthorizationError, AuthorizationRequest, ClientAuthenticationMethod, ClientRegistration,
    ProviderClient, UserInfoAuthenticationMethod,
};

/// Who the provider says signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalUser {
    /// A plain OAuth 2.0 login, with the `email` user info attribute.
    OAuth2 { email: Option<String> },
    /// An OpenID Connect login, with the claims of the verified ID token and the user info.
    Oidc {
        email: Option<String>,
        email_verified: Option<bool>,
    },
}

/// Scopes that grant access to the user info endpoint (`OidcUserService.accessibleScopes`).
const USER_INFO_SCOPES: [&str; 4] = ["profile", "email", "address", "phone"];

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    id_token: Option<String>,
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubEmail {
    email: String,
    verified: bool,
    primary: bool,
}

/// Scopes that let GitHub list the user's email addresses.
const GITHUB_EMAIL_SCOPES: [&str; 2] = ["user:email", "user"];

impl ClientRegistration {
    /// Exchanges the code of a validated callback and loads the user it belongs to.
    ///
    /// # Errors
    /// The token endpoint's own error code, or the Spring error code of the failed step.
    pub async fn sign_in(
        &self,
        client: &ProviderClient,
        request: &AuthorizationRequest,
        code: &str,
    ) -> Result<ExternalUser, AuthorizationError> {
        let tokens = self.exchange(client, request, code).await?;
        if self.is_openid() {
            let claims = self.oidc_claims(client, request, &tokens).await?;
            return Ok(ExternalUser::Oidc {
                email: claims
                    .get("email")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                email_verified: claims.get("email_verified").and_then(as_bool),
            });
        }
        let attributes = self.user_info(client, &tokens.access_token).await?;
        let mut email = attributes
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let lists_emails = self.registration_id.eq_ignore_ascii_case("github")
            && self
                .scopes
                .iter()
                .any(|scope| GITHUB_EMAIL_SCOPES.contains(&scope.as_str()));
        if email.is_none() && lists_emails {
            email = self
                .github_primary_email(client, &tokens.access_token)
                .await;
        }
        Ok(ExternalUser::OAuth2 { email })
    }

    async fn exchange(
        &self,
        client: &ProviderClient,
        request: &AuthorizationRequest,
        code: &str,
    ) -> Result<TokenResponse, AuthorizationError> {
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &request.redirect_uri),
            ("code_verifier", request.code_verifier()),
        ];
        let mut http = client.http.post(self.provider.token_uri.clone());
        match self.client_authentication_method {
            // RFC 6749 section 2.3.1: both parts are form-encoded before Basic encoding.
            ClientAuthenticationMethod::ClientSecretBasic => {
                http = http.basic_auth(
                    form_encode(&self.client_id),
                    Some(form_encode(self.client_secret.expose())),
                );
            }
            ClientAuthenticationMethod::ClientSecretPost => {
                form.push(("client_id", &self.client_id));
                form.push(("client_secret", self.client_secret.expose()));
            }
            ClientAuthenticationMethod::None => form.push(("client_id", &self.client_id)),
        }
        let body = self
            .fetch_json(http.form(&form), "token")
            .await
            .ok_or(AuthorizationError::InvalidTokenResponse)?;
        if let Some(error) = body.get("error").and_then(Value::as_str) {
            return Err(AuthorizationError::Provider(error.to_owned()));
        }
        serde_json::from_value::<TokenResponse>(body)
            .ok()
            .filter(|tokens| tokens.token_type.eq_ignore_ascii_case("bearer"))
            .ok_or(AuthorizationError::InvalidTokenResponse)
    }

    /// The claims of the verified ID token, merged over the user info claims (`OidcUserService`).
    async fn oidc_claims(
        &self,
        client: &ProviderClient,
        request: &AuthorizationRequest,
        tokens: &TokenResponse,
    ) -> Result<Claims, AuthorizationError> {
        let mut claims = self
            .id_token_claims(client, request, tokens.id_token.as_deref())
            .await?;
        if self.retrieves_user_info(tokens.scope.as_deref()) {
            let user_info = self.user_info(client, &tokens.access_token).await?;
            // OpenID Connect Core 5.3.2: user info about another subject must not be used.
            let subject = user_info.get("sub").and_then(Value::as_str);
            if subject.is_none() || subject != claims.get("sub").and_then(Value::as_str) {
                return Err(AuthorizationError::InvalidUserInfoResponse);
            }
            for (name, value) in user_info {
                claims.entry(name).or_insert(value);
            }
        }
        Ok(claims)
    }

    /// Spring asks the user info endpoint only when one is configured and the granted scopes
    /// (the requested ones when the token response names none) allow it.
    fn retrieves_user_info(&self, granted_scope: Option<&str>) -> bool {
        let granted: Vec<&str> = match granted_scope.filter(|scope| !scope.trim().is_empty()) {
            Some(scope) => scope.split_whitespace().collect(),
            None => self.scopes.iter().map(String::as_str).collect(),
        };
        self.provider.user_info_uri.is_some()
            && (granted.is_empty() || granted.iter().any(|scope| USER_INFO_SCOPES.contains(scope)))
    }

    pub(crate) async fn user_info(
        &self,
        client: &ProviderClient,
        access_token: &str,
    ) -> Result<Map<String, Value>, AuthorizationError> {
        let uri = self
            .provider
            .user_info_uri
            .clone()
            .ok_or(AuthorizationError::MissingUserInfoUri)?;
        let name = self
            .provider
            .user_name_attribute
            .as_deref()
            .ok_or(AuthorizationError::MissingUserNameAttribute)?;
        let http = match self.provider.user_info_authentication_method {
            UserInfoAuthenticationMethod::Header => client.http.get(uri).bearer_auth(access_token),
            UserInfoAuthenticationMethod::Form => client
                .http
                .post(uri)
                .form(&[("access_token", access_token)]),
        };
        match self.fetch_json(http, "user info").await {
            Some(Value::Object(attributes))
                if attributes.get(name).is_some_and(|value| !value.is_null()) =>
            {
                Ok(attributes)
            }
            _ => Err(AuthorizationError::InvalidUserInfoResponse),
        }
    }

    /// The verified primary address, or none when GitHub cannot list them.
    async fn github_primary_email(
        &self,
        client: &ProviderClient,
        access_token: &str,
    ) -> Option<String> {
        let uri = self.provider.user_info_uri.as_ref()?;
        let http = client
            .http
            .get(format!("{}/emails", uri.as_str().trim_end_matches('/')))
            .bearer_auth(access_token);
        let emails = self.fetch_json(http, "GitHub emails").await?;
        let emails: Vec<GithubEmail> = serde_json::from_value(emails)
            .inspect_err(|error| warn!(%error, "Unexpected GitHub emails response"))
            .ok()?;
        emails
            .into_iter()
            .find(|email| email.verified && email.primary)
            .map(|email| email.email)
    }
}

/// Spring converts `email_verified` with its claim conversion service, which also reads
/// strings; some providers send `"true"`.
fn as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn form_encode(value: &str) -> String {
    byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
pub(crate) mod tests;
