//! Users, server claim, OAuth2 providers, and authentication activity routes.

mod authentication_activity;
mod claim;
mod oauth2;
mod user;

use axum::Router;

use crate::routes::ApiState;

pub(super) fn claim_routes() -> Router<ApiState> {
    claim::routes()
}

pub(super) fn routes() -> Router<ApiState> {
    Router::new()
        .merge(user::routes())
        .merge(authentication_activity::routes())
}

pub(super) fn public_routes() -> Router<ApiState> {
    oauth2::routes()
}
