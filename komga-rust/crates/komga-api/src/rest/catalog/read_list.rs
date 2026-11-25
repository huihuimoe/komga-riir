//! `ReadListController` routes.

use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use komga_service::{ReadListService, ReadProgressService};
use komga_types::{BookId, LibraryId, ReadListId, ReadListSort, SortDirection, SortOrder};

use super::set_poster;
use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::media::book_archive_response;
use crate::mvc::multipart_form::MultipartForm;
use crate::mvc::pageable::{SpringPage, full_text_sort, page_request, spring_page};
use crate::mvc::produces::{Controller, JSON, OCTET_STREAM};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::{
    ReadListCreationDto, ReadListDto, ReadListRequestMatchDto, ReadListUpdateDto,
    TachiyomiReadProgressDto, TachiyomiReadProgressUpdateDto,
};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/readlists", get(list).post(create))
        .route_multipart(
            "/api/v1/readlists/match/comicrack",
            post(match_comic_rack_list),
        )
        .route(
            "/api/v1/readlists/{id}",
            get(details).patch(update).delete(delete),
        )
        .route_producing("/api/v1/readlists/{id}/file", OCTET_STREAM, get(download))
        .route("/api/v1/books/{book_id}/readlists", get(containing_book))
        .route(
            "/api/v1/readlists/{id}/read-progress/tachiyomi",
            get(tachiyomi_progress).put(mark_tachiyomi_progress),
        )
        .into_router()
        .merge(set_poster::routes::<ReadListId>("/api/v1/readlists"))
}

/// `getReadLists`: without a requested sort, by relevance when searching and by name otherwise.
async fn list(
    Authenticated(user): Authenticated,
    State(read_lists): State<ReadListService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<ReadListDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let search = params.full_text_search();
    let unpaged = params
        .boolean_or("unpaged", false)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let page = page_request(&params, unpaged);
    let by_name = [SortOrder {
        key: ReadListSort::Name,
        direction: SortDirection::Asc,
    }];
    let sort = full_text_sort(
        &params,
        read_list_sort,
        ReadListSort::Relevance,
        search.is_some(),
        &by_name,
    );
    let libraries: Option<Vec<LibraryId>> = params
        .list::<String>("library_id")
        .expect("Every value parses as a string")
        .map(|ids| ids.into_iter().map(LibraryId::from).collect());
    let result = read_lists
        .list(&user, libraries.as_deref(), search.as_deref(), &sort, page)
        .await
        .map_err(|error| ApiError::from_read_list(&error, path))?;
    Ok(spring_page(result, page, &sort))
}

/// `ReadListDao.sorts`.
fn read_list_sort(property: &str) -> Option<ReadListSort> {
    match property {
        "name" => Some(ReadListSort::Name),
        "createdDate" => Some(ReadListSort::CreatedDate),
        "lastModifiedDate" => Some(ReadListSort::LastModifiedDate),
        "relevance" => Some(ReadListSort::Relevance),
        _ => None,
    }
}

async fn tachiyomi_progress(
    Authenticated(user): Authenticated,
    State(read_progress): State<ReadProgressService>,
    Path(id): Path<ReadListId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<TachiyomiReadProgressDto>, ApiError> {
    let progress = read_progress
        .read_list_tachiyomi(&user, &id)
        .await
        .map_err(|error| ApiError::from_read_progress(&error, uri.path()))?;
    Ok(Json(progress.into()))
}

async fn mark_tachiyomi_progress(
    Authenticated(user): Authenticated,
    State(read_progress): State<ReadProgressService>,
    Path(id): Path<ReadListId>,
    OriginalUri(uri): OriginalUri,
    JsonBody(update): JsonBody<TachiyomiReadProgressUpdateDto>,
) -> Result<StatusCode, ApiError> {
    let last_book_read = update.validate()?;
    read_progress
        .mark_read_list_tachiyomi(&user, &id, last_book_read)
        .await
        .map_err(|error| ApiError::from_read_progress(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn details(
    Authenticated(user): Authenticated,
    State(read_lists): State<ReadListService>,
    Path(id): Path<ReadListId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<ReadListDto>, ApiError> {
    let read_list = read_lists
        .get(&user, &id)
        .await
        .map_err(|error| ApiError::from_read_list(&error, uri.path()))?;
    Ok(Json(read_list.into()))
}

/// `BookController.getReadListsByBookId`.
async fn containing_book(
    Authenticated(user): Authenticated,
    State(read_lists): State<ReadListService>,
    Path(book_id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<ReadListDto>>, ApiError> {
    let found = read_lists
        .containing_book(&user, &book_id)
        .await
        .map_err(|error| ApiError::from_read_list(&error, uri.path()))?;
    Ok(Json(found.into_iter().map(ReadListDto::from).collect()))
}

/// Spring validates the body before `@PreAuthorize` runs, so an invalid body is a 400 even for
/// non-admins.
async fn create(
    Authenticated(user): Authenticated,
    State(read_lists): State<ReadListService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<ReadListCreationDto>,
) -> Result<Json<ReadListDto>, ApiError> {
    ApiError::check_violations(payload.violations())?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    let read_list = read_lists
        .create(payload.into_new())
        .await
        .map_err(|error| ApiError::from_read_list(&error, uri.path()))?;
    Ok(Json(read_list.into()))
}

/// `matchComicRackList`: `file` binds before `@PreAuthorize` runs.
async fn match_comic_rack_list(
    Authenticated(user): Authenticated,
    State(read_lists): State<ReadListService>,
    OriginalUri(uri): OriginalUri,
    form: MultipartForm,
) -> Result<Json<ReadListRequestMatchDto>, ApiError> {
    let path = uri.path();
    let file = form.required_file("file", path)?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(path));
    }
    let found = read_lists
        .match_comic_rack_list(file)
        .await
        .map_err(|error| ApiError::from_read_list(&error, path))?;
    Ok(Json(found.into()))
}

async fn update(
    Authenticated(user): Authenticated,
    State(read_lists): State<ReadListService>,
    Path(id): Path<ReadListId>,
    OriginalUri(uri): OriginalUri,
    JsonBody(payload): JsonBody<ReadListUpdateDto>,
) -> Result<StatusCode, ApiError> {
    ApiError::check_violations(payload.violations())?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(uri.path()));
    }
    read_lists
        .update(&user, &id, payload.into_update())
        .await
        .map_err(|error| ApiError::from_read_list(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete(
    Admin(user): Admin,
    State(read_lists): State<ReadListService>,
    Path(id): Path<ReadListId>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    read_lists
        .delete(&user, &id)
        .await
        .map_err(|error| ApiError::from_read_list(&error, uri.path()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// `downloadReadListAsZip`.
async fn download(
    Authenticated(user): Authenticated,
    State(read_lists): State<ReadListService>,
    Path(id): Path<ReadListId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let archive = read_lists
        .archive(&user, &id)
        .await
        .map_err(|error| ApiError::from_read_list(&error, uri.path()))?;
    Ok(book_archive_response(archive))
}
