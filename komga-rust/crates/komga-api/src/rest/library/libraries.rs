//! LibraryController routes.

use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use komga_service::{BookAnalysisService, LibraryService, MetadataService};
use komga_types::LibraryId;

use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::{LibraryCreationDto, LibraryDto, LibraryUpdateDto};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    #[expect(deprecated, reason = "Kotlin still serves PUT on a library.")]
    let library = get(details)
        .patch(update)
        .put(update_deprecated)
        .delete(delete);
    Controller::new(JSON)
        .route("/api/v1/libraries", get(list).post(create))
        .route("/api/v1/libraries/{library_id}", library)
        .route("/api/v1/libraries/{library_id}/scan", post(scan))
        .route("/api/v1/libraries/{library_id}/analyze", post(analyze))
        .route(
            "/api/v1/libraries/{library_id}/empty-trash",
            post(empty_trash),
        )
        .route(
            "/api/v1/libraries/{library_id}/metadata/refresh",
            post(refresh_metadata),
        )
        .into_router()
}

async fn list(
    Authenticated(user): Authenticated,
    State(libraries): State<LibraryService>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<LibraryDto>>, ApiError> {
    let result = libraries
        .list(&user)
        .await
        .map_err(|error| ApiError::from_library(error, uri.path()))?;
    Ok(Json(
        result
            .into_iter()
            .map(|library| LibraryDto::from_library(library, user.is_admin()))
            .collect(),
    ))
}

async fn details(
    Authenticated(user): Authenticated,
    State(libraries): State<LibraryService>,
    Path(id): Path<LibraryId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<LibraryDto>, ApiError> {
    let library = libraries
        .get(&user, &id)
        .await
        .map_err(|error| ApiError::from_library(error, uri.path()))?;
    Ok(Json(LibraryDto::from_library(library, user.is_admin())))
}

async fn create(
    Authenticated(user): Authenticated,
    State(libraries): State<LibraryService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<LibraryCreationDto>,
) -> Result<Json<LibraryDto>, ApiError> {
    ApiError::check_violations(payload.violations())?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    let library = libraries
        .create(&user, payload.into_library())
        .await
        .map_err(|error| ApiError::from_library(error, uri.path()))?;
    Ok(Json(LibraryDto::from_library(library, true)))
}

async fn update(
    Authenticated(user): Authenticated,
    State(libraries): State<LibraryService>,
    Path(id): Path<LibraryId>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<LibraryUpdateDto>,
) -> Result<StatusCode, ApiError> {
    ApiError::check_violations(payload.violations())?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    libraries
        .update(&user, &id, payload.into_update())
        .await
        .map_err(|error| ApiError::from_library(error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `updateLibraryByIdDeprecated`: the same partial update as `PATCH`.
#[deprecated = "Use PATCH /api/v1/libraries/{libraryId} instead. Deprecated since 1.3.0."]
async fn update_deprecated(
    user: Authenticated,
    libraries: State<LibraryService>,
    id: Path<LibraryId>,
    uri: OriginalUri,
    payload: JsonBody<LibraryUpdateDto>,
) -> Result<StatusCode, ApiError> {
    update(user, libraries, id, uri, payload).await
}

async fn delete(
    Admin(user): Admin,
    State(libraries): State<LibraryService>,
    Path(id): Path<LibraryId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    libraries
        .delete(&user, &id)
        .await
        .map_err(|error| ApiError::from_library(error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Spring binds `deep` before `@PreAuthorize` runs, so a malformed value is a 400 even for
/// non-admins.
async fn scan(
    Authenticated(user): Authenticated,
    State(libraries): State<LibraryService>,
    Path(id): Path<LibraryId>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<StatusCode, ApiError> {
    let deep = RequestParams::parse(query.as_deref())
        .boolean_or("deep", false)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, uri.path()))?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    libraries
        .scan(&user, &id, deep)
        .await
        .map_err(|error| ApiError::from_library(error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

async fn empty_trash(
    Admin(user): Admin,
    State(libraries): State<LibraryService>,
    Path(id): Path<LibraryId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    libraries
        .empty_trash(&user, &id)
        .await
        .map_err(|error| ApiError::from_library(error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

async fn analyze(
    Admin(user): Admin,
    State(analysis): State<BookAnalysisService>,
    Path(id): Path<LibraryId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    analysis
        .submit_library(&user, &id)
        .await
        .map_err(|error| ApiError::from_book_analysis(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

async fn refresh_metadata(
    Admin(user): Admin,
    State(metadata): State<MetadataService>,
    Path(id): Path<LibraryId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    metadata
        .submit_library_refresh(&user, &id)
        .await
        .map_err(|error| ApiError::from_metadata(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}
