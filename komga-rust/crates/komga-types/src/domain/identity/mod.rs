//! User, API key, authentication activity, and content restriction types.

mod api_key;
mod authentication_activity;
pub(crate) mod content_restrictions;
mod content_scope;
mod user;

pub use api_key::ApiKey;
pub use authentication_activity::{ActivitySort, AuthenticationActivity, RecordActivityParams};
pub use content_restrictions::{AgeRestriction, AllowExclude, ContentRestrictions};
pub use content_scope::ContentScope;
pub use user::{KomgaUser, UserRole};
