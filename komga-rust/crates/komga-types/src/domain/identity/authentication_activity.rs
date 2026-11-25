//! Authentication activity domain model and recording parameters.

use jiff::civil::DateTime;

/// Record of an authentication attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticationActivity {
    pub user_id: Option<String>,
    pub email: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub success: bool,
    pub error: Option<String>,
    pub source: Option<String>,
    pub api_key_id: Option<String>,
    pub api_key_comment: Option<String>,
    /// UTC.
    pub date_time: DateTime,
}

/// The sort keys of authentication activity (`AuthenticationActivityDao.sorts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivitySort {
    DateTime,
    Email,
    Success,
    Ip,
    Error,
    UserId,
    UserAgent,
}

/// Parameters for logging an authentication event matching LoginListener.kt.
#[derive(Debug, Clone, Default)]
pub struct RecordActivityParams<'a> {
    pub user_id: Option<&'a str>,
    pub email: Option<&'a str>,
    pub source: &'a str,
    pub success: bool,
    pub error: Option<&'a str>,
    pub ip: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    pub api_key_id: Option<&'a str>,
    pub api_key_comment: Option<&'a str>,
}
