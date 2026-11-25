//! Server settings, the startup settings derived from them, and client settings.

mod client;
mod remember_me;
mod server;

use komga_db::DbError;
use thiserror::Error;

pub use client::ClientSettingsService;
pub use remember_me::RememberMeSettings;
pub use server::{ServerSettings, ServerSettingsService, ServerSettingsUpdate};

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("Database error: {0}")]
    Db(#[from] DbError),

    #[error("Invalid SERVER_SETTINGS.{key} value: {value:?}")]
    InvalidStoredValue { key: &'static str, value: String },

    #[error("Failed to generate a remember-me key: {0}")]
    Random(#[from] getrandom::Error),
}
