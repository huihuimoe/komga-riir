//! The series views of `SeriesController` and of `SeriesCollectionController`.

use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use komga_service::CatalogService;
use komga_types::{
    CollectionId, DateOperator, Equality, EqualityNullable, NumericNullable, PageRequest,
    PageResult, ReadStatus, SeriesCondition, SeriesDetails, SeriesId, SeriesSort, SeriesStatus,
    SortDirection, SortOrder,
};

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::pageable::{SpringPage, full_text_sort, page_request};
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::{GroupCountDto, SeriesDto, SeriesSearchDto};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/series/list", post(list))
        .route(
            "/api/v1/series/list/alphabetical-groups",
            post(alphabetical_groups),
        )
        .route(
            "/api/v1/series/latest",
            get(|user, catalog, uri, query| recent(user, catalog, uri, query, Recent::Latest)),
        )
        .route(
            "/api/v1/series/new",
            get(|user, catalog, uri, query| recent(user, catalog, uri, query, Recent::New)),
        )
        .route(
            "/api/v1/series/updated",
            get(|user, catalog, uri, query| recent(user, catalog, uri, query, Recent::Updated)),
        )
        .route("/api/v1/series/{series_id}", get(series))
        .route("/api/v1/collections/{id}/series", get(collection_series))
        .into_router()
}

/// `getSeriesById`.
async fn series(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    Path(series_id): Path<SeriesId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<SeriesDto>, ApiError> {
    let series = catalog
        .series(&user, &series_id)
        .await
        .map_err(|error| ApiError::from_catalog(&error, uri.path()))?;
    Ok(Json(SeriesDto::new(series, user.is_admin())))
}

/// `getSeries`: without a requested sort, by relevance when searching and unsorted otherwise;
/// collection numbers only order series when the condition selects a collection.
async fn list(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
    JsonBody(search): JsonBody<SeriesSearchDto>,
) -> Result<Json<SpringPage<SeriesDto>>, ApiError> {
    let path = uri.path();
    let condition = search.condition.as_ref();
    let full_text = full_text(&search);
    let params = RequestParams::parse(query.as_deref());
    let unpaged = params
        .boolean_or("unpaged", false)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let page = page_request(&params, unpaged);
    let in_collection = condition.is_some_and(|condition| !condition.collections().is_empty());
    let mut sort = full_text_sort(
        &params,
        series_sort,
        SeriesSort::Relevance,
        full_text.is_some(),
        &[],
    );
    sort.retain(|order| order.key != SeriesSort::CollectionNumber || in_collection);
    let result = catalog
        .series_list(&user, condition, full_text, &sort, page)
        .await
        .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(series_page(result, page, !sort.is_empty(), user.is_admin()))
}

/// Which recent series a view lists.
#[derive(Debug, Clone, Copy)]
enum Recent {
    /// Most recently modified.
    Latest,
    /// Most recently created.
    New,
    /// Most recently modified, of those modified since their creation.
    Updated,
}

/// `getSeriesLatest`/`getSeriesNew`/`getSeriesUpdated`: newest first, sort parameters ignored.
async fn recent(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
    view: Recent,
) -> Result<Json<SpringPage<SeriesDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let mismatch = |mismatch| ApiError::type_mismatch(&mismatch, path);
    let unpaged = params.boolean_or("unpaged", false).map_err(mismatch)?;
    let deleted = params.boolean("deleted").map_err(mismatch)?;
    let oneshot = params.boolean("oneshot").map_err(mismatch)?;
    let page = page_request(&params, unpaged);
    let libraries = params
        .list::<String>("library_id")
        .expect("Every value parses as a string");
    let mut filters = Vec::new();
    if let Some(ids) = libraries.filter(|ids| !ids.is_empty()) {
        let ids = ids
            .into_iter()
            .map(|id| SeriesCondition::LibraryId(Equality::Is { value: id }));
        filters.push(SeriesCondition::AnyOf(ids.collect()));
    }
    filters.extend(deleted.map(|value| SeriesCondition::Deleted(value.into())));
    filters.extend(oneshot.map(|value| SeriesCondition::OneShot(value.into())));
    let condition = (!filters.is_empty()).then_some(SeriesCondition::AllOf(filters));
    let key = match view {
        Recent::Latest | Recent::Updated => SeriesSort::LastModifiedDate,
        Recent::New => SeriesSort::CreatedDate,
    };
    let sort = [SortOrder {
        key,
        direction: SortDirection::Desc,
    }];
    let result = match view {
        Recent::Latest | Recent::New => {
            catalog
                .series_list(&user, condition.as_ref(), None, &sort, page)
                .await
        }
        Recent::Updated => {
            catalog
                .updated_series(&user, condition.as_ref(), &sort, page)
                .await
        }
    }
    .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(series_page(
        result,
        page,
        /* sorted= */ true,
        user.is_admin(),
    ))
}

/// `getSeriesAlphabeticalGroups`.
async fn alphabetical_groups(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    OriginalUri(uri): OriginalUri,
    JsonBody(search): JsonBody<SeriesSearchDto>,
) -> Result<Json<Vec<GroupCountDto>>, ApiError> {
    let groups = catalog
        .series_first_characters(&user, search.condition.as_ref(), full_text(&search))
        .await
        .map_err(|error| ApiError::from_catalog(&error, uri.path()))?;
    Ok(Json(groups.into_iter().map(GroupCountDto::from).collect()))
}

fn full_text(search: &SeriesSearchDto) -> Option<&str> {
    search
        .full_text_search
        .as_deref()
        .filter(|text| !text.trim().is_empty())
}

/// Only admins see the directories of the series.
pub(super) fn series_page(
    result: PageResult<SeriesDetails>,
    page: PageRequest,
    sorted: bool,
    admin: bool,
) -> Json<SpringPage<SeriesDto>> {
    let result = PageResult {
        items: result
            .items
            .into_iter()
            .map(|series| SeriesDto::new(series, admin))
            .collect(),
        total_elements: result.total_elements,
    };
    Json(SpringPage::new(result, page, sorted))
}

/// `getSeriesByCollectionId`: paged without sort; each filter parameter matches any of its values.
async fn collection_series(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    Path(id): Path<CollectionId>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<SeriesDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let mismatch = |mismatch| ApiError::type_mismatch(&mismatch, path);
    let unpaged = params.boolean_or("unpaged", false).map_err(mismatch)?;
    let deleted = params.boolean("deleted").map_err(mismatch)?;
    let complete = params.boolean("complete").map_err(mismatch)?;
    let page = page_request(&params, unpaged);
    let mut filters = collection_filters(&params)
        .map_err(|message| ApiError::new(StatusCode::BAD_REQUEST, message, path))?;
    filters.extend(deleted.map(|value| SeriesCondition::Deleted(value.into())));
    filters.extend(complete.map(|value| SeriesCondition::Complete(value.into())));
    let result = catalog
        .collection_series(&user, &id, filters, page)
        .await
        .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(series_page(
        result,
        page,
        /* sorted= */ true,
        user.is_admin(),
    ))
}

/// The `anyOf` conditions of the list parameters given with values.
pub(super) fn collection_filters(params: &RequestParams) -> Result<Vec<SeriesCondition>, String> {
    let strings = |name| params.list::<String>(name);
    let statuses: Option<Vec<SeriesStatus>> = params.list("status")?;
    let read_statuses: Option<Vec<ReadStatus>> = params.list("read_status")?;
    // Kotlin `toIntOrNull`: an age rating that is no integer matches unrated series; a release
    // year that is none is ignored.
    let age_ratings: Option<Vec<Option<i32>>> = strings("age_rating")?
        .map(|ratings| ratings.iter().map(|rating| rating.parse().ok()).collect());
    let release_years: Option<Vec<i32>> = strings("release_year")?
        .map(|years| years.iter().filter_map(|year| year.parse().ok()).collect());
    let filters = [
        any_of(strings("library_id")?, |id| {
            SeriesCondition::LibraryId(Equality::Is { value: id })
        }),
        any_of(statuses, |status| {
            SeriesCondition::SeriesStatus(Equality::Is { value: status })
        }),
        any_of(strings("publisher")?, |publisher| {
            SeriesCondition::Publisher(Equality::Is { value: publisher })
        }),
        any_of(strings("language")?, |language| {
            SeriesCondition::Language(Equality::Is { value: language })
        }),
        any_of(strings("tag")?, |tag| {
            SeriesCondition::Tag(EqualityNullable::Is { value: tag })
        }),
        any_of(strings("genre")?, |genre| {
            SeriesCondition::Genre(EqualityNullable::Is { value: genre })
        }),
        any_of(age_ratings, |rating| {
            SeriesCondition::AgeRating(match rating {
                Some(value) => NumericNullable::Is { value },
                None => NumericNullable::IsNull,
            })
        }),
        any_of(release_years, released_in),
        any_of(read_statuses, |status| {
            SeriesCondition::ReadStatus(Equality::Is { value: status })
        }),
        any_of(Some(params.authors()), |author| {
            SeriesCondition::Author(Equality::Is { value: author })
        }),
    ];
    Ok(filters.into_iter().flatten().collect())
}

/// The `anyOf` condition of `values`, unless there are none.
pub(super) fn any_of<T>(
    values: Option<Vec<T>>,
    condition: impl Fn(T) -> SeriesCondition,
) -> Option<SeriesCondition> {
    values
        .filter(|values| !values.is_empty())
        .map(|values| SeriesCondition::AnyOf(values.into_iter().map(condition).collect()))
}

/// Released after noon UTC on the last day of the previous year and before noon UTC on the first
/// day of the next. A year whose bounds lie past the supported dates matches no series, as no
/// stored release date can lie in it.
fn released_in(year: i32) -> SeriesCondition {
    let noon = |year: i64, month, day| -> Option<Timestamp> {
        let date = Date::new(i16::try_from(year).ok()?, month, day).ok()?;
        Some(
            date.at(12, 0, 0, 0)
                .to_zoned_datetime(TimeZone::UTC)
                .ok()?
                .timestamp(),
        )
    };
    let year = i64::from(year);
    let release_date = SeriesCondition::ReleaseDate;
    match (noon(year - 1, 12, 31), noon(year + 1, 1, 1)) {
        (Some(after), Some(before)) => SeriesCondition::AllOf(vec![
            release_date(DateOperator::After { date_time: after }),
            release_date(DateOperator::Before { date_time: before }),
        ]),
        _ => SeriesCondition::AllOf(vec![
            release_date(DateOperator::IsNull),
            release_date(DateOperator::IsNotNull),
        ]),
    }
}

/// `SeriesDtoDao.sorts`.
pub(super) fn series_sort(property: &str) -> Option<SeriesSort> {
    Some(match property {
        "metadata.titleSort" => SeriesSort::TitleSort,
        "createdDate" | "created" => SeriesSort::CreatedDate,
        "lastModifiedDate" | "lastModified" => SeriesSort::LastModifiedDate,
        "booksMetadata.releaseDate" => SeriesSort::ReleaseDate,
        "readDate" => SeriesSort::ReadDate,
        "collection.number" => SeriesSort::CollectionNumber,
        "name" => SeriesSort::Name,
        "booksCount" => SeriesSort::BooksCount,
        "random" => SeriesSort::Random,
        "relevance" => SeriesSort::Relevance,
        _ => return None,
    })
}
