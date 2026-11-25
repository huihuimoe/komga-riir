//! Book thumbnail CRUD and regeneration handlers.

use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use komga_service::ArtworkService;
use komga_types::{BookId, ThumbnailId};

use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::multipart_form::MultipartForm;
use crate::mvc::produces::{Controller, JPEG, JSON};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::ThumbnailBookDto;
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/books/thumbnails", put(regenerate_thumbnails))
        .route_producing("/api/v1/books/{book_id}/thumbnail", JPEG, get(thumbnail))
        .route("/api/v1/books/{book_id}/thumbnails", get(thumbnails))
        .route_multipart("/api/v1/books/{book_id}/thumbnails", post(add_thumbnail))
        .route_producing(
            "/api/v1/books/{book_id}/thumbnails/{thumbnail_id}",
            JPEG,
            get(thumbnail_by_id),
        )
        .route(
            "/api/v1/books/{book_id}/thumbnails/{thumbnail_id}",
            delete(delete_thumbnail),
        )
        .route(
            "/api/v1/books/{book_id}/thumbnails/{thumbnail_id}/selected",
            put(select_thumbnail),
        )
        .into_router()
}

/// Spring binds `for_bigger_result_only` before `@PreAuthorize` runs, so a malformed value is a
/// 400 even for non-admins.
async fn regenerate_thumbnails(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    RawQuery(query): RawQuery,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    let for_bigger_result_only = RequestParams::parse(query.as_deref())
        .boolean_or("for_bigger_result_only", false)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, uri.path()))?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    artwork
        .submit_regeneration(for_bigger_result_only)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

/// `getBookThumbnail`: always typed as JPEG, like the Spring `produces` value, whatever the
/// stored format.
async fn thumbnail(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let bytes = artwork
        .book_thumbnail(&user, &id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok((
        [(CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))],
        bytes,
    )
        .into_response())
}

async fn thumbnails(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<ThumbnailBookDto>>, ApiError> {
    let thumbnails = artwork
        .book_thumbnails(&user, &id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(Json(
        thumbnails
            .into_iter()
            .map(|thumbnail| ThumbnailBookDto::new(&id, thumbnail))
            .collect(),
    ))
}

/// `addUserUploadedBookThumbnail`: `file` and `selected` bind before `@PreAuthorize` runs.
async fn add_thumbnail(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
    form: MultipartForm,
) -> Result<Json<ThumbnailBookDto>, ApiError> {
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
        .add_book_thumbnail(&id, file, selected)
        .await
        .map_err(|error| ApiError::from_artwork(&error, path))?;
    Ok(Json(ThumbnailBookDto::new(&id, thumbnail)))
}

async fn thumbnail_by_id(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path((id, thumbnail_id)): Path<(BookId, ThumbnailId)>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let bytes = artwork
        .book_thumbnail_by_id(&user, &id, &thumbnail_id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok((
        [(CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))],
        bytes,
    )
        .into_response())
}

async fn select_thumbnail(
    Admin(_): Admin,
    State(artwork): State<ArtworkService>,
    Path((id, thumbnail_id)): Path<(BookId, ThumbnailId)>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    artwork
        .select_book_thumbnail(&id, &thumbnail_id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

async fn delete_thumbnail(
    Admin(_): Admin,
    State(artwork): State<ArtworkService>,
    Path((id, thumbnail_id)): Path<(BookId, ThumbnailId)>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    artwork
        .delete_book_thumbnail(&id, &thumbnail_id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}
