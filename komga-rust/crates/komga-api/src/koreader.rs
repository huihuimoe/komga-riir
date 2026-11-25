//! Kotlin `KoreaderSyncController`: the KOReader progress sync server under `/koreader`.

use axum::extract::{OriginalUri, Path, State};
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use komga_service::{KoreaderError, KoreaderProgress, KoreaderSyncService};
use komga_types::R2Device;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::auth::authenticate_koreader;
use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::produces::Controller;
use crate::routes::ApiState;

/// Kotlin `DocumentProgressDto`.
#[derive(Serialize, Deserialize)]
struct DocumentProgressDto {
    /// The KOReader partial MD5 of the book file.
    document: String,
    percentage: f32,
    progress: String,
    device: String,
    device_id: String,
}

const PRODUCES: &[&str] = &["application/json", "application/vnd.koreader.v1+json"];

pub fn router(state: &ApiState) -> Router {
    Controller::new(PRODUCES)
        .route("/koreader/users/auth", get(authorize))
        .route("/koreader/syncs/progress/{hash}", get(progress))
        .route("/koreader/syncs/progress", put(update_progress))
        .into_router()
        .route_layer(from_fn_with_state(
            state.auth.clone(),
            authenticate_koreader,
        ))
        // Anyone may ask, whatever they accept; nobody may register.
        .route("/koreader/users/create", post(register))
        .with_state(state.koreader.clone())
}

async fn register(OriginalUri(uri): OriginalUri) -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "User creation is disabled",
        uri.path(),
    )
}

async fn authorize() -> Json<serde_json::Value> {
    Json(json!({"authorized": "OK"}))
}

/// The user's progress, or `{}` when they have none (Kotlin: a Spring error body with status
/// 200).
async fn progress(
    State(service): State<KoreaderSyncService>,
    Authenticated(user): Authenticated,
    Path(hash): Path<String>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let progress = service
        .progress(&user, &hash)
        .await
        .map_err(|error| from_koreader(&error, uri.path()))?;
    Ok(match progress {
        Some(progress) => Json(DocumentProgressDto {
            document: hash,
            percentage: progress.percentage,
            progress: progress.progress,
            device: progress.device.name,
            device_id: progress.device.id,
        })
        .into_response(),
        None => Json(json!({})).into_response(),
    })
}

async fn update_progress(
    State(service): State<KoreaderSyncService>,
    Authenticated(user): Authenticated,
    OriginalUri(uri): OriginalUri,
    JsonBody(body): JsonBody<DocumentProgressDto>,
) -> Result<StatusCode, ApiError> {
    let update = KoreaderProgress {
        percentage: body.percentage,
        progress: body.progress,
        device: R2Device {
            id: body.device_id,
            name: body.device,
        },
    };
    service
        .update_progress(&user, &body.document, update)
        .await
        .map_err(|error| from_koreader(&error, uri.path()))?;
    Ok(StatusCode::OK)
}

fn from_koreader(error: &KoreaderError, path: &str) -> ApiError {
    match error {
        KoreaderError::NotFound | KoreaderError::NoProfile => {
            ApiError::new(StatusCode::NOT_FOUND, error.to_string(), path)
        }
        KoreaderError::Conflict => ApiError::new(StatusCode::CONFLICT, error.to_string(), path),
        KoreaderError::BadProgress => ApiError::new(StatusCode::BAD_REQUEST, "Bad Request", path),
        KoreaderError::Progress(error) => ApiError::from_read_progress(error, path),
        KoreaderError::Db(_) => ApiError::internal_error(path, error),
    }
}
