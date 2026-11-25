//! OAuth 2.0 and OpenID Connect login: provider registrations, the authorization request, and
//! the protocol checks of the callback.

mod authorization;
mod builder;
mod client;
mod discovery;
mod mapper;
mod oidc;
mod outbound;
mod properties;
mod registration;
mod sign_in;

pub use authorization::{
    AuthorizationError, AuthorizationRequest, AuthorizationResponse, FLOW_TTL,
};
pub use client::ProviderClient;
pub use discovery::DiscoveryError;
pub use outbound::http_client;
pub use properties::{
    OAuth2ClientProperties, ProviderProperties, RegistrationError, RegistrationProperties,
};
pub use registration::{
    ClientAuthenticationMethod, ClientRegistration, ClientSecret, ProviderDetails,
    UserInfoAuthenticationMethod,
};
pub use sign_in::ExternalUser;
