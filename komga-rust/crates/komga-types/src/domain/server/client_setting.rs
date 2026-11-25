//! Client settings stored for web clients (`CLIENT_SETTINGS_GLOBAL`).

/// A global client setting; only those allowing unauthorized access are visible anonymously.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalClientSetting {
    pub value: String,
    pub allow_unauthorized: bool,
}
