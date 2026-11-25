//! OAuth2Controller: the login providers the sign-in page offers.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use komga_service::ExternalIdentityService;
use serde::Serialize;

use crate::mvc::produces::{Controller, JSON};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/oauth2/providers", get(providers))
        .into_router()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OAuth2ClientDto {
    name: String,
    registration_id: String,
}

/// `getProviders`.
async fn providers(State(external): State<ExternalIdentityService>) -> Json<Vec<OAuth2ClientDto>> {
    let providers = external
        .registrations()
        .iter()
        .map(|registration| OAuth2ClientDto {
            name: registration.client_name.clone(),
            registration_id: registration.registration_id.clone(),
        })
        .collect();
    Json(providers)
}
