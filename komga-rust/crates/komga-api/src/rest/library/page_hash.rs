//! `PageHashController` routes, but for deleting duplicate pages.
//!
//! Parameters and bodies are checked before the role, as Spring binds them before
//! `@PreAuthorize` runs.

use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use komga_service::PageHashService;
use komga_types::{KnownPageHashSort, PageHashAction, PageHashMatchSort, UnknownPageHashSort};

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::media::media_type_or_default;
use crate::mvc::pageable::{SpringPage, keyed, page_request, spring_page};
use crate::mvc::produces::{Controller, JPEG, JSON};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::{
    PageHashCreationDto, PageHashKnownDto, PageHashMatchDto, PageHashUnknownDto,
};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route(
            "/api/v1/page-hashes",
            get(known).put(create_or_update_known),
        )
        .route("/api/v1/page-hashes/unknown", get(unknown))
        .route("/api/v1/page-hashes/{page_hash}", get(matches))
        .route_producing(
            "/api/v1/page-hashes/{page_hash}/thumbnail",
            JPEG,
            get(known_thumbnail),
        )
        .route(
            "/api/v1/page-hashes/{page_hash}/delete-all",
            post(delete_all),
        )
        .route(
            "/api/v1/page-hashes/{page_hash}/delete-match",
            post(delete_match),
        )
        .route_producing(
            "/api/v1/page-hashes/unknown/{page_hash}/thumbnail",
            JPEG,
            get(unknown_thumbnail),
        )
        .into_router()
}

async fn known(
    Authenticated(user): Authenticated,
    State(page_hashes): State<PageHashService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<PageHashKnownDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let actions: Option<Vec<PageHashAction>> = params
        .list("action")
        .map_err(|message| bad_request(message, path))?;
    let page = page_request(&params, /* unpaged= */ false);
    let sort = keyed(&params, known_sort);
    admin_only(user.is_admin(), path)?;
    let result = page_hashes
        .known(actions.as_deref(), &sort, page)
        .await
        .map_err(|error| ApiError::from_page_hash(&error, path))?;
    Ok(spring_page(result, page, &sort))
}

async fn unknown(
    Authenticated(user): Authenticated,
    State(page_hashes): State<PageHashService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<PageHashUnknownDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let page = page_request(&params, /* unpaged= */ false);
    let sort = keyed(&params, unknown_sort);
    admin_only(user.is_admin(), path)?;
    let result = page_hashes
        .unknown(&sort, page)
        .await
        .map_err(|error| ApiError::from_page_hash(&error, path))?;
    Ok(spring_page(result, page, &sort))
}

async fn matches(
    Authenticated(user): Authenticated,
    State(page_hashes): State<PageHashService>,
    Path(page_hash): Path<String>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<PageHashMatchDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let page = page_request(&params, /* unpaged= */ false);
    let sort = keyed(&params, match_sort);
    admin_only(user.is_admin(), path)?;
    let result = page_hashes
        .matches(&page_hash, &sort, page)
        .await
        .map_err(|error| ApiError::from_page_hash(&error, path))?;
    Ok(spring_page(result, page, &sort))
}

/// Typed as JPEG, like the Spring `produces` value; it is the stored 500-pixel thumbnail.
async fn known_thumbnail(
    Authenticated(user): Authenticated,
    State(page_hashes): State<PageHashService>,
    Path(page_hash): Path<String>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let path = uri.path();
    admin_only(user.is_admin(), path)?;
    let bytes = page_hashes
        .known_thumbnail(&page_hash)
        .await
        .map_err(|error| ApiError::from_page_hash(&error, path))?
        .ok_or_else(|| not_found(path))?;
    Ok((
        [(CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))],
        bytes,
    )
        .into_response())
}

/// The first page with the hash, as stored or resized to `resize` pixels.
async fn unknown_thumbnail(
    Authenticated(user): Authenticated,
    State(page_hashes): State<PageHashService>,
    Path(page_hash): Path<String>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let path = uri.path();
    let resize = RequestParams::parse(query.as_deref())
        .positive_integer("resize")
        .map_err(|message| bad_request(message, path))?;
    admin_only(user.is_admin(), path)?;
    let content = page_hashes
        .page(&page_hash, resize)
        .await
        .map_err(|error| ApiError::from_page_hash(&error, path))?
        .ok_or_else(|| not_found(path))?;
    Ok((
        [(
            CONTENT_TYPE,
            media_type_or_default(Some(&content.media_type)),
        )],
        content.bytes,
    )
        .into_response())
}

async fn create_or_update_known(
    Authenticated(user): Authenticated,
    State(page_hashes): State<PageHashService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<PageHashCreationDto>,
) -> Result<StatusCode, ApiError> {
    let path = uri.path();
    ApiError::check_violations(payload.violations())?;
    admin_only(user.is_admin(), path)?;
    page_hashes
        .create_or_update(&payload.hash, payload.size, payload.action)
        .await
        .map_err(|error| ApiError::from_page_hash(&error, path))?;
    Ok(StatusCode::ACCEPTED)
}

async fn delete_all(
    Authenticated(user): Authenticated,
    State(page_hashes): State<PageHashService>,
    OriginalUri(uri): OriginalUri,
    Path(page_hash): Path<String>,
) -> Result<StatusCode, ApiError> {
    let path = uri.path();
    admin_only(user.is_admin(), path)?;
    page_hashes
        .delete_all(&page_hash)
        .await
        .map_err(|error| ApiError::from_page_hash(&error, path))?;
    Ok(StatusCode::ACCEPTED)
}

async fn delete_match(
    Authenticated(user): Authenticated,
    State(page_hashes): State<PageHashService>,
    OriginalUri(uri): OriginalUri,
    Path(page_hash): Path<String>,
    JsonBody(payload): JsonBody<PageHashMatchDto>,
) -> Result<StatusCode, ApiError> {
    let path = uri.path();
    admin_only(user.is_admin(), path)?;
    let (book_id, page) = payload.into_page(&page_hash);
    page_hashes
        .delete_match(book_id, page)
        .await
        .map_err(|error| ApiError::from_page_hash(&error, path))?;
    Ok(StatusCode::ACCEPTED)
}

fn admin_only(is_admin: bool, path: &str) -> Result<(), ApiError> {
    if is_admin {
        Ok(())
    } else {
        Err(ApiError::forbidden(path))
    }
}

fn bad_request(message: String, path: &str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, message, path)
}

/// `ResponseStatusException(HttpStatus.NOT_FOUND)`.
fn not_found(path: &str) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path)
}

/// `PageHashDao.sortsKnown`.
fn known_sort(property: &str) -> Option<KnownPageHashSort> {
    match property {
        "hash" => Some(KnownPageHashSort::Hash),
        "matchCount" => Some(KnownPageHashSort::MatchCount),
        "deleteCount" => Some(KnownPageHashSort::DeleteCount),
        "deleteSize" => Some(KnownPageHashSort::DeleteSize),
        "fileSize" | "size" => Some(KnownPageHashSort::Size),
        "createdDate" | "created" => Some(KnownPageHashSort::Created),
        "lastModifiedDate" | "lastModified" => Some(KnownPageHashSort::LastModified),
        _ => None,
    }
}

/// `PageHashDao.sortsUnknown`, but for the columns the query lacks.
fn unknown_sort(property: &str) -> Option<UnknownPageHashSort> {
    match property {
        "hash" => Some(UnknownPageHashSort::Hash),
        "fileSize" | "size" => Some(UnknownPageHashSort::Size),
        "matchCount" => Some(UnknownPageHashSort::MatchCount),
        "totalSize" => Some(UnknownPageHashSort::TotalSize),
        _ => None,
    }
}

/// `PageHashDao.sortsUnknown`, which Kotlin applies to matches too, for the columns they have.
fn match_sort(property: &str) -> Option<PageHashMatchSort> {
    match property {
        "fileSize" | "size" => Some(PageHashMatchSort::Size),
        "url" => Some(PageHashMatchSort::Url),
        "bookId" => Some(PageHashMatchSort::BookId),
        "pageNumber" => Some(PageHashMatchSort::PageNumber),
        _ => None,
    }
}
