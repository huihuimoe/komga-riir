//! Signing in through an OAuth 2.0 or OpenID Connect provider.

mod flow_keys;

pub use flow_keys::{FlowKeyRing, FlowSecret};

use std::sync::Arc;
use std::time::SystemTime;

use komga_db::{Database, DbError};
use komga_oauth::{
    AuthorizationError, AuthorizationRequest, AuthorizationResponse, ClientRegistration,
    ExternalUser, ProviderClient,
};
use komga_types::{ContentRestrictions, KomgaUser, UserRole};
use thiserror::Error;
use tracing::info;

use crate::identity::{IdentityError, IdentityService, LibrarySharing, NewUser};
use flow_keys::FlowKeys;

#[derive(Debug, Error)]
pub enum ExternalIdentityError {
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error("Failed to generate an OAuth2 flow key: {0}")]
    Random(#[from] getrandom::Error),
    // The value is a secret and stays out of the message.
    #[error("Invalid SERVER_SETTINGS.{0} value")]
    InvalidStoredKey(&'static str),
}

/// Why a callback signs nobody in.
#[derive(Debug, Error)]
pub enum SignInError {
    #[error(transparent)]
    Protocol(#[from] AuthorizationError),
    /// Komga refuses the provider's user; the code is listed in `ERRORCODES.md`.
    #[error("{0}")]
    Refused(&'static str),
    #[error(transparent)]
    Identity(#[from] IdentityError),
}

impl SignInError {
    /// The code the login page shows, or none for a server failure.
    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        match self {
            Self::Protocol(error) => Some(error.error_code()),
            Self::Refused(code) => Some(code),
            Self::Identity(_) => None,
        }
    }
}

/// How users of the providers become Komga users (`komga.oauth2-account-creation` and
/// `komga.oidc-email-verification`).
#[derive(Debug, Clone, Copy)]
pub struct SignInPolicy {
    pub account_creation: bool,
    pub oidc_email_verification: bool,
}

/// The configured login providers and the keys of logins in progress.
#[derive(Clone)]
pub struct ExternalIdentityService {
    registrations: Arc<[ClientRegistration]>,
    flow_keys: FlowKeys,
    client: ProviderClient,
    identity: IdentityService,
    policy: SignInPolicy,
}

impl ExternalIdentityService {
    /// # Errors
    /// Fails on database errors and on a stored flow key that cannot be read.
    pub async fn load(
        database: Database,
        identity: IdentityService,
        registrations: Vec<ClientRegistration>,
        policy: SignInPolicy,
    ) -> Result<Self, ExternalIdentityError> {
        Ok(Self {
            registrations: registrations.into(),
            flow_keys: FlowKeys::load(database).await?,
            client: ProviderClient::new(),
            identity,
            policy,
        })
    }

    /// The providers a user may sign in with, in registration id order.
    pub fn registrations(&self) -> &[ClientRegistration] {
        &self.registrations
    }

    pub fn registration(&self, registration_id: &str) -> Option<&ClientRegistration> {
        self.registrations
            .iter()
            .find(|registration| registration.registration_id == registration_id)
    }

    /// The keys that seal and open the flow cookie, replacing an expired one first.
    ///
    /// # Errors
    /// Fails when a replaced key cannot be stored.
    pub async fn flow_keys(&self) -> Result<FlowKeyRing, ExternalIdentityError> {
        self.flow_keys.ring().await
    }

    /// Completes a login that came back to the callback of `registration` and returns the
    /// Komga user with the provider user's email, created first when the policy allows it.
    ///
    /// # Errors
    /// Returns the error to show on the login page, or a database failure.
    pub async fn complete(
        &self,
        registration: &ClientRegistration,
        request: Option<&AuthorizationRequest>,
        response: &AuthorizationResponse,
    ) -> Result<KomgaUser, SignInError> {
        let (request, code) = AuthorizationRequest::validate(
            request,
            &registration.registration_id,
            response,
            SystemTime::now(),
        )?;
        let external = registration.sign_in(&self.client, request, &code).await?;
        let email = self.policy.email(external)?;
        match self.identity.find_by_email(&email).await? {
            Some(user) => Ok(user),
            None if self.policy.account_creation => self.create_user(email).await,
            None => Err(SignInError::Refused("ERR_1025")),
        }
    }

    /// `KomgaOAuth2UserServiceConfiguration.tryCreateNewUser`; a concurrent callback for the same
    /// email may create the user first, which then signs in.
    async fn create_user(&self, email: String) -> Result<KomgaUser, SignInError> {
        let new_user = NewUser {
            email: email.clone(),
            password: random_password(),
            roles: [UserRole::FileDownload, UserRole::PageStreaming].into(),
            library_sharing: LibrarySharing::default(),
            restrictions: ContentRestrictions::default(),
        };
        match self.identity.create_user(new_user).await {
            Ok(user) => {
                info!(email = %user.email, "Created a user for an OAuth2 login");
                Ok(user)
            }
            Err(IdentityError::UserAlreadyExists(_)) => self
                .identity
                .find_by_email(&email)
                .await?
                .ok_or(SignInError::Refused("ERR_1025")),
            Err(error) => Err(error.into()),
        }
    }
}

impl SignInPolicy {
    /// The email that identifies the provider user (`KomgaOAuth2UserServiceConfiguration`).
    fn email(self, external: ExternalUser) -> Result<String, SignInError> {
        match external {
            ExternalUser::OAuth2 { email } => email.ok_or(SignInError::Refused("ERR_1024")),
            ExternalUser::Oidc {
                email,
                email_verified,
            } => {
                let email = email.ok_or(SignInError::Refused("ERR_1028"))?;
                match email_verified {
                    _ if !self.oidc_email_verification => Ok(email),
                    Some(true) => Ok(email),
                    Some(false) => Err(SignInError::Refused("ERR_1026")),
                    None => Err(SignInError::Refused("ERR_1027")),
                }
            }
        }
    }
}

/// Twelve random letters and digits, which nobody knows: the user signs in through the provider.
fn random_password() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).expect("Failed to generate random bytes");
    bytes
        .iter()
        .map(|byte| char::from(ALPHABET[usize::from(*byte) % ALPHABET.len()]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oidc(email: Option<&str>, email_verified: Option<bool>) -> ExternalUser {
        ExternalUser::Oidc {
            email: email.map(str::to_owned),
            email_verified,
        }
    }

    #[test]
    fn the_email_policy_follows_komga_error_codes() {
        let verifying = SignInPolicy {
            account_creation: false,
            oidc_email_verification: true,
        };
        let trusting = SignInPolicy {
            oidc_email_verification: false,
            ..verifying
        };
        let email = Some("reader@example.org");
        let cases = [
            (
                verifying,
                ExternalUser::OAuth2 { email: None },
                Err("ERR_1024"),
            ),
            (verifying, oidc(None, Some(true)), Err("ERR_1028")),
            (verifying, oidc(email, Some(false)), Err("ERR_1026")),
            (verifying, oidc(email, None), Err("ERR_1027")),
            (verifying, oidc(email, Some(true)), Ok("reader@example.org")),
            (trusting, oidc(email, Some(false)), Ok("reader@example.org")),
            (trusting, oidc(None, None), Err("ERR_1028")),
        ];
        for (policy, external, expected) in cases {
            let actual = policy.email(external.clone());
            let actual = actual
                .as_deref()
                .map_err(|error| error.error_code().unwrap_or_default());
            assert_eq!(actual, expected, "{external:?}");
        }
    }
}
