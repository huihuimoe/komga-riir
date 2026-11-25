//! User and API key REST routes.
//!
//! Spring binds and validates `@RequestBody` before `@PreAuthorize` runs, so handlers with a body
//! check their authorization rule after validation.

use axum::extract::{OriginalUri, Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use komga_types::{ApiKeyId, UserId};

use crate::auth::AuthState;
use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::produces::{Controller, JSON};
use crate::rest::dto::{
    ApiKeyDto, ApiKeyRequestDto, PasswordUpdateDto, UserCreationDto, UserDto, UserUpdateDto,
};
use crate::routes::ApiState;

/// Create routes for `/api/v2/users`.
pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v2/users", get(list_users))
        .route("/api/v2/users/me", get(get_me))
        .route("/api/v2/users", post(create_user))
        .route("/api/v2/users/{id}", patch(update_user))
        .route("/api/v2/users/{id}", delete(delete_user))
        .route("/api/v2/users/{id}/password", patch(update_password_by_id))
        .route(
            "/api/v2/users/me/password",
            patch(update_current_user_password),
        )
        .route("/api/v2/users/me/api-keys", get(get_my_api_keys))
        .route("/api/v2/users/me/api-keys", post(create_my_api_key))
        .route(
            "/api/v2/users/me/api-keys/{key_id}",
            delete(delete_my_api_key),
        )
        .into_router()
}

async fn get_me(Authenticated(user): Authenticated) -> Json<UserDto> {
    Json(UserDto::from(&*user))
}

async fn list_users(
    _admin: Admin,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<UserDto>>, ApiError> {
    let users = state
        .identity()
        .list_users()
        .await
        .map_err(|error| ApiError::from_identity(error, uri.path()))?;
    Ok(Json(users.into_iter().map(UserDto::from).collect()))
}

async fn create_user(
    Authenticated(current_user): Authenticated,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<UserCreationDto>,
) -> Result<(StatusCode, Json<UserDto>), ApiError> {
    ApiError::check_violations(payload.violations())?;
    if !current_user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    match state.identity().create_user(payload.into_new_user()).await {
        Ok(user) => Ok((StatusCode::CREATED, Json(UserDto::from(user)))),
        Err(error) => Err(ApiError::from_identity(error, uri.path())),
    }
}

async fn update_user(
    Authenticated(current_user): Authenticated,
    Path(id): Path<UserId>,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<UserUpdateDto>,
) -> Result<StatusCode, ApiError> {
    ApiError::check_violations(payload.violations())?;
    if !current_user.is_admin() || current_user.id.as_str() == id.as_str() {
        return Err(ApiError::forbidden(uri.path()));
    }
    match state
        .identity()
        .update_user(id.as_str(), payload.into_update())
        .await
    {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(error) => Err(ApiError::from_identity(error, uri.path())),
    }
}

async fn delete_user(
    Admin(admin): Admin,
    Path(id): Path<UserId>,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    match state
        .identity()
        .delete_user(id.as_str(), admin.id.as_str())
        .await
    {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(error) => Err(ApiError::from_identity(error, uri.path())),
    }
}

async fn update_password_by_id(
    Authenticated(current_user): Authenticated,
    Path(id): Path<UserId>,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<PasswordUpdateDto>,
) -> Result<StatusCode, ApiError> {
    ApiError::check_violations(payload.violations())?;
    if !current_user.is_admin() && current_user.id.as_str() != id.as_str() {
        return Err(ApiError::forbidden(uri.path()));
    }

    let expire_sessions = current_user.id.as_str() != id.as_str();
    match state
        .identity()
        .update_password(id.as_str(), &payload.password, expire_sessions)
        .await
    {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(error) => Err(ApiError::from_identity(error, uri.path())),
    }
}

async fn update_current_user_password(
    Authenticated(current_user): Authenticated,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<PasswordUpdateDto>,
) -> Result<StatusCode, ApiError> {
    ApiError::check_violations(payload.violations())?;

    match state
        .identity()
        .update_password(current_user.id.as_str(), &payload.password, false)
        .await
    {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(error) => Err(ApiError::from_identity(error, uri.path())),
    }
}

// -----------------------------------------------------------------------------
// API Key endpoints
// -----------------------------------------------------------------------------

async fn get_my_api_keys(
    Authenticated(user): Authenticated,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<ApiKeyDto>>, ApiError> {
    let keys = state
        .identity()
        .list_api_keys(user.id.as_str())
        .await
        .map_err(|error| ApiError::from_identity(error, uri.path()))?;
    Ok(Json(
        keys.into_iter()
            .map(ApiKeyDto::from)
            .map(ApiKeyDto::redacted)
            .collect(),
    ))
}

async fn create_my_api_key(
    Authenticated(user): Authenticated,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<ApiKeyRequestDto>,
) -> Result<Json<ApiKeyDto>, ApiError> {
    ApiError::check_violations(payload.violations())?;

    match state
        .identity()
        .create_api_key(user.id.as_str(), &payload.comment)
        .await
    {
        Ok(api_key) => Ok(Json(ApiKeyDto::from(api_key))),
        Err(error) => Err(ApiError::from_identity(error, uri.path())),
    }
}

async fn delete_my_api_key(
    Authenticated(user): Authenticated,
    Path(key_id): Path<ApiKeyId>,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    match state
        .identity()
        .delete_api_key(key_id.as_str(), user.id.as_str())
        .await
    {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(error) => Err(ApiError::from_identity(error, uri.path())),
    }
}
