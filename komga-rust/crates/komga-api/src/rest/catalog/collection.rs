//! `SeriesCollectionController` routes.

use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use komga_service::SeriesCollectionService;
use komga_types::{CollectionId, CollectionSort, LibraryId, SeriesId, SortDirection, SortOrder};

use super::set_poster;
use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::pageable::{SpringPage, full_text_sort, page_request, spring_page};
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::{CollectionCreationDto, CollectionDto, CollectionUpdateDto};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/collections", get(list).post(create))
        .route(
            "/api/v1/collections/{id}",
            get(details).patch(update).delete(delete),
        )
        .route(
            "/api/v1/series/{series_id}/collections",
            get(containing_series),
        )
        .into_router()
        .merge(set_poster::routes::<CollectionId>("/api/v1/collections"))
}

/// `getCollections`: without a requested sort, by relevance when searching and by name
/// otherwise.
async fn list(
    Authenticated(user): Authenticated,
    State(collections): State<SeriesCollectionService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<CollectionDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let search = params.full_text_search();
    let unpaged = params
        .boolean_or("unpaged", false)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let page = page_request(&params, unpaged);
    let by_name = [SortOrder {
        key: CollectionSort::Name,
        direction: SortDirection::Asc,
    }];
    let sort = full_text_sort(
        &params,
        collection_sort,
        CollectionSort::Relevance,
        search.is_some(),
        &by_name,
    );
    let libraries: Option<Vec<LibraryId>> = params
        .list::<String>("library_id")
        .expect("Every value parses as a string")
        .map(|ids| ids.into_iter().map(LibraryId::from).collect());
    let result = collections
        .list(&user, libraries.as_deref(), search.as_deref(), &sort, page)
        .await
        .map_err(|error| ApiError::from_collection(&error, path))?;
    Ok(spring_page(result, page, &sort))
}

/// `SeriesCollectionDao.sorts`.
fn collection_sort(property: &str) -> Option<CollectionSort> {
    match property {
        "name" => Some(CollectionSort::Name),
        "createdDate" => Some(CollectionSort::CreatedDate),
        "lastModifiedDate" => Some(CollectionSort::LastModifiedDate),
        "relevance" => Some(CollectionSort::Relevance),
        _ => None,
    }
}

/// `SeriesController.getCollectionsBySeriesId`.
async fn containing_series(
    Authenticated(user): Authenticated,
    State(collections): State<SeriesCollectionService>,
    Path(series_id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<CollectionDto>>, ApiError> {
    let found = collections
        .containing_series(&user, &series_id)
        .await
        .map_err(|error| ApiError::from_collection(&error, uri.path()))?;
    Ok(Json(found.into_iter().map(CollectionDto::from).collect()))
}

async fn details(
    Authenticated(user): Authenticated,
    State(collections): State<SeriesCollectionService>,
    Path(id): Path<CollectionId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<CollectionDto>, ApiError> {
    let collection = collections
        .get(&user, &id)
        .await
        .map_err(|error| ApiError::from_collection(&error, uri.path()))?;
    Ok(Json(collection.into()))
}

/// Spring validates the body before `@PreAuthorize` runs, so an invalid body is a 400 even for
/// non-admins.
async fn create(
    Authenticated(user): Authenticated,
    State(collections): State<SeriesCollectionService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<CollectionCreationDto>,
) -> Result<Json<CollectionDto>, ApiError> {
    ApiError::check_violations(payload.violations())?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    let collection = collections
        .create(payload.into_new())
        .await
        .map_err(|error| ApiError::from_collection(&error, uri.path()))?;
    Ok(Json(collection.into()))
}

async fn update(
    Authenticated(user): Authenticated,
    State(collections): State<SeriesCollectionService>,
    Path(id): Path<CollectionId>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<CollectionUpdateDto>,
) -> Result<StatusCode, ApiError> {
    ApiError::check_violations(payload.violations())?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    collections
        .update(&user, &id, payload.into_update())
        .await
        .map_err(|error| ApiError::from_collection(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete(
    Admin(user): Admin,
    State(collections): State<SeriesCollectionService>,
    Path(id): Path<CollectionId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    collections
        .delete(&user, &id)
        .await
        .map_err(|error| ApiError::from_collection(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}
