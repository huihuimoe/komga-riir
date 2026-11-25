//! Authentication metadata extracted from HTTP request headers.

use axum::http::{HeaderMap, header};

/// Metadata extracted from request for logging authentication activities.
#[derive(Debug, Clone, Default)]
pub struct AuthActivityMetadata {
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

impl AuthActivityMetadata {
    /// Extract IP via `client_ip` extractors (x_forwarded_for, x_real_ip, cf, etc.)
    /// and User-Agent from HTTP headers.
    #[must_use]
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let ip = client_ip::rightmost_x_forwarded_for(headers)
            .or_else(|_| client_ip::x_real_ip(headers))
            .or_else(|_| client_ip::cf_connecting_ip(headers))
            .ok()
            .map(|ip| ip.to_string());

        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        Self { ip, user_agent }
    }
}
