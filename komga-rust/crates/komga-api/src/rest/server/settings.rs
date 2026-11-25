//! Server settings routes (`SettingsController`).

use axum::extract::{OriginalUri, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use komga_service::ServerSettingsService;

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::multipart_form::MAX_FILE_BYTES;
use crate::mvc::produces::{Controller, JSON};
use crate::rest::dto::{SettingMultiSource, SettingsDto, SettingsUpdateDto};
use crate::routes::{ApiState, WebServerSettings};

/// Create routes for `/api/v1/settings`.
pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route(
            "/api/v1/settings",
            get(get_server_settings).patch(update_server_settings),
        )
        .into_router()
}

async fn get_server_settings(
    Authenticated(user): Authenticated,
    State(settings): State<ServerSettingsService>,
    State(web_server): State<WebServerSettings>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<SettingsDto>, ApiError> {
    if !user.is_admin() {
        return Ok(Json(SettingsDto {
            max_upload_file_size_bytes: Some(max_upload_file_size_bytes()),
            ..SettingsDto::default()
        }));
    }
    let settings = settings
        .load()
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(Json(SettingsDto {
        delete_empty_collections: Some(settings.delete_empty_collections),
        delete_empty_read_lists: Some(settings.delete_empty_read_lists),
        remember_me_duration_days: Some(settings.remember_me_duration_days.into()),
        thumbnail_size: Some(settings.thumbnail_size.into()),
        task_pool_size: Some(settings.task_pool_size),
        server_port: Some(SettingMultiSource {
            configuration_source: Some(web_server.configured_port.into()),
            database_source: settings.server_port,
            effective_value: Some(web_server.effective_port.into()),
        }),
        server_context_path: Some(SettingMultiSource {
            configuration_source: web_server.configured_context_path,
            database_source: settings.server_context_path,
            effective_value: Some(web_server.context_path.to_string()),
        }),
        kobo_proxy: Some(settings.kobo_proxy),
        kobo_port: settings.kobo_port,
        // `komga.kobo.kepubify-path` is rejected at startup, so `KepubConverter` would use the
        // stored path whether or not it is executable.
        kepubify_path: Some(SettingMultiSource {
            configuration_source: None,
            database_source: settings.kepubify_path.clone(),
            effective_value: settings.kepubify_path,
        }),
        max_upload_file_size_bytes: Some(max_upload_file_size_bytes()),
        next_ui_as_default: Some(settings.next_ui_as_default),
    }))
}

async fn update_server_settings(
    Authenticated(user): Authenticated,
    State(settings): State<ServerSettingsService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<SettingsUpdateDto>,
) -> Result<StatusCode, ApiError> {
    ApiError::check_violations(payload.violations())?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    settings
        .update(payload.into_update())
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(StatusCode::NO_CONTENT)
}

fn max_upload_file_size_bytes() -> i64 {
    i64::try_from(MAX_FILE_BYTES).expect("1 MiB fits in i64")
}
