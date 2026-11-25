//! A resolved client registration (Spring Security `ClientRegistration`).

use std::fmt;

use url::Url;

/// The value of `scope` that turns an authorization code login into an OpenID Connect one.
const OPENID_SCOPE: &str = "openid";

/// One login provider the server trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientRegistration {
    /// The id in `/oauth2/authorization/{registrationId}` and the callback path.
    pub registration_id: String,
    pub client_id: String,
    /// Empty for public clients.
    pub client_secret: ClientSecret,
    pub client_authentication_method: ClientAuthenticationMethod,
    /// The callback URI template, with Spring's `{baseUrl}`, `{action}` and `{registrationId}`
    /// placeholders.
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    /// The label offered on the login page.
    pub client_name: String,
    pub provider: ProviderDetails,
}

impl ClientRegistration {
    /// Whether the login follows OpenID Connect and receives an ID token, as Spring decides it.
    #[must_use]
    pub fn is_openid(&self) -> bool {
        self.scopes.iter().any(|scope| scope == OPENID_SCOPE)
    }
}

/// The provider endpoints of a registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDetails {
    pub authorization_uri: Url,
    pub token_uri: Url,
    pub user_info_uri: Option<Url>,
    pub user_info_authentication_method: UserInfoAuthenticationMethod,
    pub user_name_attribute: Option<String>,
    pub jwk_set_uri: Option<Url>,
    /// The issuer an ID token must name; set from `issuer-uri` or the discovered metadata.
    pub issuer_uri: Option<String>,
}

/// How the client authenticates to the token endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientAuthenticationMethod {
    ClientSecretBasic,
    ClientSecretPost,
    /// A public client, which relies on PKCE alone.
    None,
}

impl ClientAuthenticationMethod {
    /// Parses Spring's configuration value. The JWT-based methods are not supported.
    #[must_use]
    pub fn from_config(value: &str) -> Option<Self> {
        match value {
            "client_secret_basic" => Some(Self::ClientSecretBasic),
            "client_secret_post" => Some(Self::ClientSecretPost),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

/// How the access token reaches the user info endpoint. Spring sends it in a form body for
/// `form` and as a bearer header for anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UserInfoAuthenticationMethod {
    #[default]
    Header,
    Form,
}

impl UserInfoAuthenticationMethod {
    #[must_use]
    pub fn from_config(value: &str) -> Self {
        if value.eq_ignore_ascii_case("form") {
            Self::Form
        } else {
            Self::Header
        }
    }
}

/// A client secret, kept out of `Debug` output and therefore out of logs.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ClientSecret(String);

impl ClientSecret {
    #[must_use]
    pub fn new(secret: String) -> Self {
        Self(secret)
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for ClientSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(if self.0.is_empty() { "\"\"" } else { "***" })
    }
}
