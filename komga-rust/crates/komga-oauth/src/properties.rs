//! Spring Boot's `spring.security.oauth2.client` properties.

use std::collections::BTreeMap;

use crate::discovery::DiscoveryError;
use crate::registration::ClientSecret;

/// `spring.security.oauth2.client.*`, keyed by registration and provider id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OAuth2ClientProperties {
    pub registration: BTreeMap<String, RegistrationProperties>,
    pub provider: BTreeMap<String, ProviderProperties>,
}

/// `spring.security.oauth2.client.registration.<id>.*`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistrationProperties {
    /// The provider id; defaults to the registration id.
    pub provider: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<ClientSecret>,
    pub client_authentication_method: Option<String>,
    pub authorization_grant_type: Option<String>,
    pub redirect_uri: Option<String>,
    pub scope: Option<Vec<String>>,
    pub client_name: Option<String>,
}

/// `spring.security.oauth2.client.provider.<id>.*`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderProperties {
    pub authorization_uri: Option<String>,
    pub token_uri: Option<String>,
    pub user_info_uri: Option<String>,
    pub user_info_authentication_method: Option<String>,
    pub user_name_attribute: Option<String>,
    pub jwk_set_uri: Option<String>,
    pub issuer_uri: Option<String>,
}

/// Why a registration cannot be built. Each one stops the server from starting, as in Spring.
#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("client id of registration '{0}' must not be empty")]
    MissingClientId(String),
    #[error("unknown provider ID '{provider}' in registration '{registration}'")]
    UnknownProvider {
        registration: String,
        provider: String,
    },
    #[error("provider ID must be specified for client registration '{0}'")]
    MissingProvider(String),
    #[error("registration '{registration}': {field} cannot be empty")]
    Missing {
        registration: String,
        field: &'static str,
    },
    #[error("registration '{registration}': {field} \"{value}\" is not supported")]
    Unsupported {
        registration: String,
        field: &'static str,
        value: String,
    },
    #[error("registration '{registration}': {field} \"{value}\" is not a valid URL")]
    InvalidUrl {
        registration: String,
        field: &'static str,
        value: String,
    },
    #[error("registration '{registration}': scope \"{scope}\" contains invalid characters")]
    InvalidScope { registration: String, scope: String },
    #[error("registration '{registration}' cannot resolve its issuer")]
    Discovery {
        registration: String,
        #[source]
        source: DiscoveryError,
    },
}
