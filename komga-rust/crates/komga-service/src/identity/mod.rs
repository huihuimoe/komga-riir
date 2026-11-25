//! Identity use cases for users, API keys, and authentication activity.

mod activity;
mod api_keys;
mod claim;
pub(crate) mod external;
mod session;
mod users;

pub use session::{DEFAULT_SESSION_TTL, SessionRegistry};
pub use users::{LibrarySharing, NewUser, UserUpdate};

use komga_db::{Database, DbError};
use komga_types::{DomainEvent, UserId};
use thiserror::Error;
use tokio::task::{self, JoinError};

use crate::events::EventBus;

const BCRYPT_COST: u32 = 10;

/// Errors arising from identity operations.
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("Database error: {0}")]
    Db(#[from] DbError),

    #[error("Server has already been claimed")]
    AlreadyClaimed,

    #[error("User with email '{0}' already exists")]
    UserAlreadyExists(String),

    #[error("API key comment already exists for this user")]
    ApiKeyCommentDuplicate,

    #[error("User not found")]
    UserNotFound,

    #[error("API key not found")]
    ApiKeyNotFound,

    #[error("Cannot delete own account")]
    CannotDeleteSelf,

    #[error("Password hashing error: {0}")]
    PasswordHash(#[from] bcrypt::BcryptError),

    #[error("Password worker failed: {0}")]
    Worker(#[from] JoinError),
}

/// Service handling user lifecycle, passwords, API keys, and server claiming.
#[derive(Clone)]
pub struct IdentityService {
    db: Database,
    sessions: SessionRegistry,
    events: EventBus,
}

impl IdentityService {
    /// Create a new identity service.
    #[must_use]
    pub fn new(db: Database, sessions: SessionRegistry, events: EventBus) -> Self {
        Self {
            db,
            sessions,
            events,
        }
    }

    /// Removes expired in-memory sessions.
    pub fn evict_expired_sessions(&self) {
        self.sessions.evict_expired();
    }

    /// Revokes the user's sessions, then tells their clients.
    fn expire_sessions(&self, user_id: &str) {
        self.sessions.invalidate_user(user_id);
        self.events
            .publish(DomainEvent::SessionsExpired(UserId::from(user_id)));
    }
}

/// Offloads CPU-intensive bcrypt hashing to Tokio's blocking pool.
pub(super) async fn hash_password(password: String) -> Result<String, IdentityError> {
    let hash = task::spawn_blocking(move || bcrypt::hash(password, BCRYPT_COST)).await??;
    Ok(hash)
}

/// Offloads CPU-intensive bcrypt verification to Tokio's blocking pool.
pub(super) async fn verify_password(password: String, hash: String) -> Result<bool, IdentityError> {
    let valid = task::spawn_blocking(move || bcrypt::verify(password, &hash)).await??;
    Ok(valid)
}
