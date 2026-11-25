//! REST routes for initial server claim.

use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use komga_service::IdentityService;

use crate::error::ApiError;
use crate::mvc::produces::{Controller, JSON};
use crate::rest::dto::{ClaimStatusDto, UserDto};
use crate::rest::validation;
use crate::routes::ApiState;

/// Create routes for `/api/v1/claim`.
pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/claim", get(get_claim_status).post(claim_server))
        .into_router()
}

async fn get_claim_status(
    State(identity): State<IdentityService>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<ClaimStatusDto>, ApiError> {
    let is_claimed = identity
        .is_claimed()
        .await
        .map_err(|error| ApiError::from_identity(error, uri.path()))?;

    Ok(Json(ClaimStatusDto { is_claimed }))
}

async fn claim_server(
    State(identity): State<IdentityService>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<UserDto>), ApiError> {
    let path = uri.path();
    let email = headers
        .get("X-Komga-Email")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "Missing or invalid X-Komga-Email header",
                path,
            )
        })?;

    let password = headers
        .get("X-Komga-Password")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "Missing or invalid X-Komga-Password header",
                path,
            )
        })?;

    // Spring method validation names the violations after the parameters.
    let mut violations = Vec::new();
    validation::email("claimServer.email", email, &mut violations);
    validation::not_blank("claimServer.password", password, &mut violations);
    ApiError::check_violations(violations)?;

    match identity.claim_server(email, password).await {
        Ok(user) => Ok((StatusCode::OK, Json(UserDto::from(user)))),
        Err(error) => Err(ApiError::from_identity(error, path)),
    }
}
