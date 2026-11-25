//! `ClientRegistration.Builder` and Spring Security's `CommonOAuth2Provider` templates.

use url::Url;

use crate::discovery::{self, ProviderMetadata};
use crate::properties::{ProviderProperties, RegistrationError, RegistrationProperties};
use crate::registration::{
    ClientAuthenticationMethod, ClientRegistration, ClientSecret, ProviderDetails,
    UserInfoAuthenticationMethod,
};

pub(crate) const AUTHORIZATION_CODE: &str = "authorization_code";
pub(crate) const DEFAULT_REDIRECT_URI: &str = "{baseUrl}/{action}/oauth2/code/{registrationId}";
pub(crate) const SUB: &str = "sub";

/// Spring matches the `CommonOAuth2Provider` names leniently: case and separators are ignored.
pub(crate) fn common_provider(provider_id: &str) -> Option<fn(&str) -> Builder> {
    let canonical: String = provider_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|ch| ch.to_ascii_lowercase())
        .collect();
    match canonical.as_str() {
        "google" => Some(google),
        "github" => Some(github),
        "facebook" => Some(facebook),
        "okta" => Some(okta),
        _ => None,
    }
}

fn common(id: &str, method: ClientAuthenticationMethod, scopes: &[&str], name: &str) -> Builder {
    Builder {
        client_authentication_method: Some(method),
        authorization_grant_type: Some(AUTHORIZATION_CODE.to_owned()),
        redirect_uri: Some(DEFAULT_REDIRECT_URI.to_owned()),
        scopes: scopes.iter().map(|scope| (*scope).to_owned()).collect(),
        client_name: Some(name.to_owned()),
        ..Builder::new(id)
    }
}

fn google(id: &str) -> Builder {
    let method = ClientAuthenticationMethod::ClientSecretBasic;
    Builder {
        authorization_uri: Some("https://accounts.google.com/o/oauth2/v2/auth".to_owned()),
        token_uri: Some("https://www.googleapis.com/oauth2/v4/token".to_owned()),
        jwk_set_uri: Some("https://www.googleapis.com/oauth2/v3/certs".to_owned()),
        issuer_uri: Some("https://accounts.google.com".to_owned()),
        user_info_uri: Some("https://www.googleapis.com/oauth2/v3/userinfo".to_owned()),
        user_name_attribute: Some(SUB.to_owned()),
        ..common(id, method, &["openid", "profile", "email"], "Google")
    }
}

fn github(id: &str) -> Builder {
    let method = ClientAuthenticationMethod::ClientSecretBasic;
    Builder {
        authorization_uri: Some("https://github.com/login/oauth/authorize".to_owned()),
        token_uri: Some("https://github.com/login/oauth/access_token".to_owned()),
        user_info_uri: Some("https://api.github.com/user".to_owned()),
        user_name_attribute: Some("id".to_owned()),
        ..common(id, method, &["read:user"], "GitHub")
    }
}

fn facebook(id: &str) -> Builder {
    let method = ClientAuthenticationMethod::ClientSecretPost;
    Builder {
        authorization_uri: Some("https://www.facebook.com/v2.8/dialog/oauth".to_owned()),
        token_uri: Some("https://graph.facebook.com/v2.8/oauth/access_token".to_owned()),
        user_info_uri: Some("https://graph.facebook.com/me?fields=id,name,email".to_owned()),
        user_name_attribute: Some("id".to_owned()),
        ..common(id, method, &["public_profile", "email"], "Facebook")
    }
}

fn okta(id: &str) -> Builder {
    let method = ClientAuthenticationMethod::ClientSecretBasic;
    Builder {
        user_name_attribute: Some(SUB.to_owned()),
        ..common(id, method, &["openid", "profile", "email"], "Okta")
    }
}

/// `ClientRegistration.Builder`: later sources override earlier ones field by field.
#[derive(Default)]
pub(crate) struct Builder {
    registration_id: String,
    client_id: Option<String>,
    client_secret: Option<ClientSecret>,
    client_authentication_method: Option<ClientAuthenticationMethod>,
    authorization_grant_type: Option<String>,
    redirect_uri: Option<String>,
    scopes: Vec<String>,
    client_name: Option<String>,
    authorization_uri: Option<String>,
    token_uri: Option<String>,
    user_info_uri: Option<String>,
    user_info_authentication_method: UserInfoAuthenticationMethod,
    user_name_attribute: Option<String>,
    jwk_set_uri: Option<String>,
    issuer_uri: Option<String>,
    /// A configured method Spring would accept but komga-rust cannot use.
    unsupported_method: Option<String>,
}

impl Builder {
    pub(crate) fn new(id: &str) -> Self {
        Self {
            registration_id: id.to_owned(),
            ..Self::default()
        }
    }

    pub(crate) fn from_metadata(id: &str, issuer: &str, metadata: ProviderMetadata) -> Self {
        Self {
            client_authentication_method: discovery::token_endpoint_method(
                metadata.token_endpoint_auth_methods_supported.as_deref(),
            ),
            authorization_grant_type: Some(AUTHORIZATION_CODE.to_owned()),
            redirect_uri: Some(DEFAULT_REDIRECT_URI.to_owned()),
            client_name: Some(issuer.to_owned()),
            authorization_uri: metadata.authorization_endpoint,
            token_uri: Some(metadata.token_endpoint),
            user_info_uri: metadata.userinfo_endpoint,
            user_name_attribute: Some(SUB.to_owned()),
            jwk_set_uri: metadata.jwks_uri,
            issuer_uri: Some(issuer.to_owned()),
            ..Self::new(id)
        }
    }

    pub(crate) fn apply_provider(&mut self, provider: &ProviderProperties) {
        set(&mut self.authorization_uri, &provider.authorization_uri);
        set(&mut self.token_uri, &provider.token_uri);
        set(&mut self.user_info_uri, &provider.user_info_uri);
        if let Some(method) = &provider.user_info_authentication_method {
            self.user_info_authentication_method =
                UserInfoAuthenticationMethod::from_config(method);
        }
        set(&mut self.jwk_set_uri, &provider.jwk_set_uri);
        set(&mut self.user_name_attribute, &provider.user_name_attribute);
        set(&mut self.issuer_uri, &provider.issuer_uri);
    }

    pub(crate) fn apply_registration(&mut self, registration: &RegistrationProperties) {
        set(&mut self.client_id, &registration.client_id);
        set(&mut self.client_secret, &registration.client_secret);
        if let Some(method) = &registration.client_authentication_method {
            match ClientAuthenticationMethod::from_config(method) {
                Some(method) => self.client_authentication_method = Some(method),
                None => self.unsupported_method = Some(method.clone()),
            }
        }
        set(
            &mut self.authorization_grant_type,
            &registration.authorization_grant_type,
        );
        set(&mut self.redirect_uri, &registration.redirect_uri);
        if let Some(scope) = &registration.scope {
            self.scopes.clone_from(scope);
        }
        set(&mut self.client_name, &registration.client_name);
    }

    pub(crate) fn build(self) -> Result<ClientRegistration, RegistrationError> {
        let id = self.registration_id;
        let missing = |field| RegistrationError::Missing {
            registration: id.clone(),
            field,
        };
        let unsupported = |field, value: String| RegistrationError::Unsupported {
            registration: id.clone(),
            field,
            value,
        };
        let grant_type = self
            .authorization_grant_type
            .ok_or_else(|| missing("authorizationGrantType"))?;
        if grant_type != AUTHORIZATION_CODE {
            return Err(unsupported("authorizationGrantType", grant_type));
        }
        if let Some(method) = self.unsupported_method {
            return Err(unsupported("clientAuthenticationMethod", method));
        }
        let client_id = has_text(self.client_id).ok_or_else(|| missing("clientId"))?;
        let redirect_uri = has_text(self.redirect_uri).ok_or_else(|| missing("redirectUri"))?;
        let authorization_uri =
            has_text(self.authorization_uri).ok_or_else(|| missing("authorizationUri"))?;
        let token_uri = has_text(self.token_uri).ok_or_else(|| missing("tokenUri"))?;
        if let Some(scope) = self.scopes.iter().find(|scope| !is_valid_scope(scope)) {
            return Err(RegistrationError::InvalidScope {
                registration: id.clone(),
                scope: scope.clone(),
            });
        }
        let client_secret = self
            .client_secret
            .filter(|secret| !secret.expose().trim().is_empty())
            .unwrap_or_default();
        let client_authentication_method =
            self.client_authentication_method
                .unwrap_or(if client_secret.is_empty() {
                    ClientAuthenticationMethod::None
                } else {
                    ClientAuthenticationMethod::ClientSecretBasic
                });
        let url = |field, value: String| {
            Url::parse(&value).map_err(|_| RegistrationError::InvalidUrl {
                registration: id.clone(),
                field,
                value,
            })
        };
        let provider = ProviderDetails {
            authorization_uri: url("authorizationUri", authorization_uri)?,
            token_uri: url("tokenUri", token_uri)?,
            user_info_uri: has_text(self.user_info_uri)
                .map(|value| url("userInfoUri", value))
                .transpose()?,
            user_info_authentication_method: self.user_info_authentication_method,
            user_name_attribute: has_text(self.user_name_attribute),
            jwk_set_uri: has_text(self.jwk_set_uri)
                .map(|value| url("jwkSetUri", value))
                .transpose()?,
            issuer_uri: has_text(self.issuer_uri),
        };
        Ok(ClientRegistration {
            client_name: has_text(self.client_name).unwrap_or_else(|| id.clone()),
            registration_id: id,
            client_id,
            client_secret,
            client_authentication_method,
            redirect_uri,
            scopes: self.scopes,
            provider,
        })
    }
}

fn set<T: Clone>(target: &mut Option<T>, value: &Option<T>) {
    if let Some(value) = value {
        *target = Some(value.clone());
    }
}

fn has_text(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

/// RFC 6749 §3.3: `scope-token = 1*( %x21 / %x23-5B / %x5D-7E )`.
fn is_valid_scope(scope: &str) -> bool {
    !scope.is_empty()
        && scope
            .bytes()
            .all(|byte| matches!(byte, 0x21 | 0x23..=0x5b | 0x5d..=0x7e))
}
