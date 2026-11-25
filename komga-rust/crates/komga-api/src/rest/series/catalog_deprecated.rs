//! The deprecated `GET` series lists of `SeriesController`, which query parameters filter.

use axum::extract::{OriginalUri, RawQuery, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use komga_service::CatalogService;
use komga_types::{
    Equality, EqualityNullable, SeriesCondition, SeriesSort, TitleField, TitleRegex,
};

use super::catalog::{any_of, collection_filters, series_page, series_sort};
use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::pageable::{SpringPage, full_text_sort, page_request};
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::{GroupCountDto, SeriesDto};
use crate::routes::ApiState;

/// Routes of the deprecated series lists, for the caller to authenticate.
#[deprecated = "Use the POST /api/v1/series/list routes instead."]
#[expect(
    deprecated,
    reason = "Registers the deprecated handlers that Kotlin still serves."
)]
pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/series", get(series))
        .route(
            "/api/v1/series/alphabetical-groups",
            get(alphabetical_groups),
        )
        .into_router()
}

/// `getSeriesDeprecated`: without a requested sort, by relevance when searching and unsorted
/// otherwise; collection numbers only order series when `collection_id` selects a collection.
#[deprecated = "Use POST /api/v1/series/list instead. Deprecated since 1.19.0."]
async fn series(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<SeriesDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let condition = condition(&params, path)?;
    let full_text = params.full_text_search();
    let unpaged = params
        .boolean_or("unpaged", false)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let page = page_request(&params, unpaged);
    let in_collection = !condition.collections().is_empty();
    let mut sort = full_text_sort(
        &params,
        series_sort,
        SeriesSort::Relevance,
        full_text.is_some(),
        &[],
    );
    sort.retain(|order| order.key != SeriesSort::CollectionNumber || in_collection);
    let result = catalog
        .series_list(&user, Some(&condition), full_text.as_deref(), &sort, page)
        .await
        .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(series_page(result, page, !sort.is_empty(), user.is_admin()))
}

/// `getSeriesAlphabeticalGroupsDeprecated`.
#[deprecated = "Use POST /api/v1/series/list/alphabetical-groups instead. Deprecated since 1.19.0."]
async fn alphabetical_groups(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<Vec<GroupCountDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let condition = condition(&params, path)?;
    let groups = catalog
        .series_first_characters(
            &user,
            Some(&condition),
            params.full_text_search().as_deref(),
        )
        .await
        .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(Json(groups.into_iter().map(GroupCountDto::from).collect()))
}

/// The series the query parameters select; each list parameter matches any of its values.
fn condition(params: &RequestParams, path: &str) -> Result<SeriesCondition, ApiError> {
    let mismatch = |mismatch| ApiError::type_mismatch(&mismatch, path);
    let deleted = params.boolean("deleted").map_err(mismatch)?;
    let complete = params.boolean("complete").map_err(mismatch)?;
    let oneshot = params.boolean("oneshot").map_err(mismatch)?;
    let bad_request = |message| ApiError::new(StatusCode::BAD_REQUEST, message, path);
    let mut filters = collection_filters(params).map_err(bad_request)?;
    let strings = |name| {
        params
            .list::<String>(name)
            .expect("Every value parses as a string")
    };
    filters.extend(any_of(strings("collection_id"), |id| {
        SeriesCondition::CollectionId(Equality::Is { value: id })
    }));
    filters.extend(any_of(strings("sharing_label"), |label| {
        SeriesCondition::SharingLabel(EqualityNullable::Is { value: label })
    }));
    filters.extend(oneshot.map(|value| SeriesCondition::OneShot(value.into())));
    filters.extend(complete.map(|value| SeriesCondition::Complete(value.into())));
    filters.extend(deleted.map(|value| SeriesCondition::Deleted(value.into())));
    filters.extend(
        title_regex(params)
            .map_err(bad_request)?
            .map(SeriesCondition::TitleRegex),
    );
    Ok(SeriesCondition::AllOf(filters))
}

/// `@DelimitedPair("search_regex")`: the first value split at its last comma into a pattern and
/// a field, `title` or `title_sort` in any case; without a comma or with another field the
/// parameter is ignored.
fn title_regex(params: &RequestParams) -> Result<Option<TitleRegex>, String> {
    let Some((pattern, field)) = params
        .first("search_regex")
        .and_then(|value| value.rsplit_once(','))
    else {
        return Ok(None);
    };
    let field = match field.to_lowercase().as_str() {
        "title" => TitleField::Title,
        "title_sort" => TitleField::TitleSort,
        _ => return Ok(None),
    };
    TitleRegex::try_new(pattern.to_owned(), field)
        .map(Some)
        .map_err(|error| format!("Invalid value for parameter 'search_regex': {error}"))
}
