//! Client settings routes (`ClientSettingsController`).
//!
//! The baseline checks `@PreAuthorize` before the method validation of keys and values: both are
//! AOP advisors on the controller proxy, and the method-security proxy is created first. Request
//! bodies are still parsed before either.

use std::collections::BTreeMap;

use axum::extract::{OriginalUri, State};
use axum::http::StatusCode;
use axum::routing::{get, patch};
use axum::{Json, Router};
use komga_service::ClientSettingsService;
use komga_types::GlobalClientSetting;

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::produces::{Controller, JSON};
use crate::rest::dto::{
    ClientSettingDto, ClientSettingGlobalUpdateDto, ClientSettingKeys, ClientSettingUserUpdateDto,
    ClientSettingsUpdate, validate_keys, validate_update,
};
use crate::routes::ApiState;

type SettingsResponse = Result<Json<BTreeMap<String, ClientSettingDto>>, ApiError>;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/client-settings/user/list", get(get_user_settings))
        .route(
            "/api/v1/client-settings/global",
            patch(save_global_settings).delete(delete_global_settings),
        )
        .route(
            "/api/v1/client-settings/user",
            patch(save_user_settings).delete(delete_user_settings),
        )
        .into_router()
}

pub(super) fn public_routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route(
            "/api/v1/client-settings/global/list",
            get(get_global_settings),
        )
        .into_router()
}

async fn get_global_settings(
    user: Option<Authenticated>,
    State(settings): State<ClientSettingsService>,
    OriginalUri(uri): OriginalUri,
) -> SettingsResponse {
    let global = settings
        .global_settings(user.as_deref())
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(Json(
        global
            .into_iter()
            .map(|(key, setting)| (key, setting.into()))
            .collect(),
    ))
}

async fn get_user_settings(
    Authenticated(user): Authenticated,
    State(settings): State<ClientSettingsService>,
    OriginalUri(uri): OriginalUri,
) -> SettingsResponse {
    let user_settings = settings
        .user_settings(&user.id)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(Json(
        user_settings
            .into_iter()
            .map(|(key, value)| {
                let setting = ClientSettingDto {
                    value,
                    allow_unauthorized: None,
                };
                (key, setting)
            })
            .collect(),
    ))
}

async fn save_global_settings(
    Authenticated(user): Authenticated,
    State(settings): State<ClientSettingsService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<ClientSettingsUpdate<ClientSettingGlobalUpdateDto>>,
) -> Result<StatusCode, ApiError> {
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    let new_settings: BTreeMap<String, GlobalClientSetting> =
        validate_update("saveGlobalSetting", payload, |setting| &setting.value)
            .map_err(ApiError::Validation)?
            .into_iter()
            .map(|(key, setting)| (key, setting.into()))
            .collect();
    settings
        .save_global(&new_settings)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn save_user_settings(
    Authenticated(user): Authenticated,
    State(settings): State<ClientSettingsService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<ClientSettingsUpdate<ClientSettingUserUpdateDto>>,
) -> Result<StatusCode, ApiError> {
    let new_settings: BTreeMap<String, String> =
        validate_update("saveUserSetting", payload, |setting| &setting.value)
            .map_err(ApiError::Validation)?
            .into_iter()
            .map(|(key, setting)| (key, setting.value))
            .collect();
    settings
        .save_for_user(&user.id, &new_settings)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_global_settings(
    Authenticated(user): Authenticated,
    State(settings): State<ClientSettingsService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<ClientSettingKeys>,
) -> Result<StatusCode, ApiError> {
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    let keys = validate_keys("deleteGlobalSettings", payload).map_err(ApiError::Validation)?;
    settings
        .delete_global(&keys)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_user_settings(
    Authenticated(user): Authenticated,
    State(settings): State<ClientSettingsService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<ClientSettingKeys>,
) -> Result<StatusCode, ApiError> {
    let keys = validate_keys("deleteUserSettings", payload).map_err(ApiError::Validation)?;
    settings
        .delete_for_user(&user.id, &keys)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(StatusCode::NO_CONTENT)
}
