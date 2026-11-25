//! REST controllers and their DTOs.

pub(crate) mod book;
mod catalog;
pub(crate) mod dto;
mod identity;
mod library;
mod series;
pub(crate) mod server;
mod validation;

use axum::Router;
use axum::middleware::from_fn_with_state;

use crate::auth::{authenticate_request, permit_anonymous};
use crate::routes::ApiState;

pub fn router(state: &ApiState) -> Router {
    let authenticated = authenticated_routes()
        .route_layer(from_fn_with_state(state.auth.clone(), authenticate_request));
    let anonymous =
        anonymous_routes().route_layer(from_fn_with_state(state.auth.clone(), permit_anonymous));
    Router::new()
        .merge(authenticated)
        .merge(anonymous)
        .merge(identity::claim_routes())
        .with_state(state.clone())
}

fn authenticated_routes() -> Router<ApiState> {
    Router::new()
        .merge(book::routes())
        .merge(series::routes())
        .merge(library::routes())
        .merge(catalog::routes())
        .merge(identity::routes())
        .merge(server::routes())
}

fn anonymous_routes() -> Router<ApiState> {
    Router::new()
        .merge(book::public_routes())
        .merge(identity::public_routes())
        .merge(server::public_routes())
}
