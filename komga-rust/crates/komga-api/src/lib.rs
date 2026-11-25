//! HTTP API, DTOs, and protocol routing for Komga.

mod accept;
mod actuator;
mod auth;
mod error;
mod kobo;
mod koreader;
mod mvc;
mod opds;
mod rest;
mod routes;
mod sse;
mod web_ui;
mod webpub;

pub use actuator::{BuildInfo, GitInfo, ServerControl};
pub use auth::{AuthState, REMEMBER_ME_COOKIE_NAME, SESSION_COOKIE_NAME};
pub use kobo::KoboStore;
pub use mvc::servlet_uri::{LocalAddress, TrustedProxies};
pub use routes::{ApiServices, WebServerSettings, router};
pub use sse::events::SseConnections;
pub use web_ui::{WebAsset, WebUi};
