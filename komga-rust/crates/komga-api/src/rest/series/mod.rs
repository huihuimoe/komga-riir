//! SeriesController routes.

mod catalog;
mod catalog_deprecated;

use axum::extract::{OriginalUri, Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use komga_service::{
    ArtworkService, BookAnalysisService, MetadataService, ReadProgressService, ReadingService,
};
use komga_types::{SeriesId, ThumbnailId};

use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::media::{book_archive_response, jpeg};
use crate::mvc::multipart_form::MultipartForm;
use crate::mvc::produces::{Controller, JPEG, JSON, OCTET_STREAM};
use crate::rest::dto::{
    SeriesMetadataUpdateDto, TachiyomiReadProgressUpdateV2Dto, TachiyomiReadProgressV2Dto,
    ThumbnailSeriesDto,
};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    #[expect(deprecated, reason = "Kotlin still serves the GET series lists.")]
    let catalog_deprecated = catalog_deprecated::routes();

    Controller::new(JSON)
        .route_producing("/api/v1/series/{series_id}/thumbnail", JPEG, get(thumbnail))
        .route("/api/v1/series/{series_id}/thumbnails", get(thumbnails))
        .route_multipart("/api/v1/series/{series_id}/thumbnails", post(add_thumbnail))
        .route_producing(
            "/api/v1/series/{series_id}/thumbnails/{thumbnail_id}",
            JPEG,
            get(thumbnail_by_id),
        )
        .route(
            "/api/v1/series/{series_id}/thumbnails/{thumbnail_id}",
            delete(delete_thumbnail),
        )
        .route(
            "/api/v1/series/{series_id}/thumbnails/{thumbnail_id}/selected",
            put(select_thumbnail),
        )
        .route("/api/v1/series/{series_id}/analyze", post(analyze))
        .route_producing(
            "/api/v1/series/{series_id}/file",
            OCTET_STREAM,
            get(download),
        )
        .route(
            "/api/v1/series/{series_id}/metadata",
            patch(update_metadata),
        )
        .route(
            "/api/v1/series/{series_id}/metadata/refresh",
            post(refresh_metadata),
        )
        .route(
            "/api/v1/series/{series_id}/read-progress",
            post(mark_read).delete(mark_unread),
        )
        .route(
            "/api/v2/series/{series_id}/read-progress/tachiyomi",
            get(tachiyomi_progress).put(mark_tachiyomi_progress),
        )
        .into_router()
        .merge(catalog::routes())
        .merge(catalog_deprecated)
}

async fn analyze(
    Admin(user): Admin,
    State(analysis): State<BookAnalysisService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    analysis
        .submit_series(&user, &id)
        .await
        .map_err(|error| ApiError::from_book_analysis(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

/// `downloadSeriesAsZip`.
async fn download(
    Authenticated(user): Authenticated,
    State(reading): State<ReadingService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let archive = reading
        .series_archive(&user, &id)
        .await
        .map_err(|error| ApiError::from_reading(&error, uri.path()))?;
    Ok(book_archive_response(archive))
}

/// Spring validates the body before `@PreAuthorize` runs, so an invalid body is a 400 even for
/// non-admins.
async fn update_metadata(
    Authenticated(user): Authenticated,
    State(metadata): State<MetadataService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<SeriesMetadataUpdateDto>,
) -> Result<StatusCode, ApiError> {
    ApiError::check_violations(payload.violations())?;
    metadata
        .update_series_metadata(&user, &id, payload.into_update())
        .await
        .map_err(|error| ApiError::from_metadata(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn mark_read(
    Authenticated(user): Authenticated,
    State(read_progress): State<ReadProgressService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    read_progress
        .mark_series_completed(&user, &id)
        .await
        .map_err(|error| ApiError::from_read_progress(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn mark_unread(
    Authenticated(user): Authenticated,
    State(read_progress): State<ReadProgressService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    read_progress
        .delete_series(&user, &id)
        .await
        .map_err(|error| ApiError::from_read_progress(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn tachiyomi_progress(
    Authenticated(user): Authenticated,
    State(read_progress): State<ReadProgressService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<TachiyomiReadProgressV2Dto>, ApiError> {
    let progress = read_progress
        .series_tachiyomi(&user, &id)
        .await
        .map_err(|error| ApiError::from_read_progress(&error, uri.path()))?;
    Ok(Json(progress.into()))
}

async fn mark_tachiyomi_progress(
    Authenticated(user): Authenticated,
    State(read_progress): State<ReadProgressService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
    JsonBody(update): JsonBody<TachiyomiReadProgressUpdateV2Dto>,
) -> Result<StatusCode, ApiError> {
    read_progress
        .mark_series_tachiyomi(&user, &id, update.last_book_number_sort_read)
        .await
        .map_err(|error| ApiError::from_read_progress(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn refresh_metadata(
    Admin(user): Admin,
    State(metadata): State<MetadataService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    metadata
        .submit_series_refresh(&user, &id)
        .await
        .map_err(|error| ApiError::from_metadata(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

async fn thumbnail(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let bytes = artwork
        .series_thumbnail(&user, &id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(jpeg(bytes))
}

async fn thumbnails(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<ThumbnailSeriesDto>>, ApiError> {
    let thumbnails = artwork
        .series_thumbnails(&user, &id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(Json(
        thumbnails
            .into_iter()
            .map(|thumbnail| ThumbnailSeriesDto::new(&id, thumbnail))
            .collect(),
    ))
}

async fn thumbnail_by_id(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path((id, thumbnail_id)): Path<(SeriesId, ThumbnailId)>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let bytes = artwork
        .series_thumbnail_by_id(&user, &id, &thumbnail_id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(jpeg(bytes))
}

/// `addUserUploadedSeriesThumbnail`: `file` and `selected` bind before `@PreAuthorize` runs.
async fn add_thumbnail(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
    form: MultipartForm,
) -> Result<Json<ThumbnailSeriesDto>, ApiError> {
    let path = uri.path();
    let file = form.required_file("file", path)?;
    let selected = form
        .params()
        .boolean_or("selected", true)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(path));
    }
    let thumbnail = artwork
        .add_series_thumbnail(&id, file, selected)
        .await
        .map_err(|error| ApiError::from_artwork(&error, path))?;
    Ok(Json(ThumbnailSeriesDto::new(&id, thumbnail)))
}

async fn select_thumbnail(
    Admin(_): Admin,
    State(artwork): State<ArtworkService>,
    Path((id, thumbnail_id)): Path<(SeriesId, ThumbnailId)>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    artwork
        .select_series_thumbnail(&id, &thumbnail_id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

async fn delete_thumbnail(
    Admin(_): Admin,
    State(artwork): State<ArtworkService>,
    Path((id, thumbnail_id)): Path<(SeriesId, ThumbnailId)>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    artwork
        .delete_series_thumbnail(&id, &thumbnail_id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}
