//! Deprecated referential routes (`ReferentialV1Controller`): unpaged lists over the queries of
//! the v2 routes, so content restrictions apply to them too.

use axum::extract::{OriginalUri, RawQuery, State};
use axum::routing::{MethodRouter, get};
use axum::{Json, Router};
use komga_service::{Credit, ReferentialService, SeriesValue};
use komga_types::{
    BookAuthor, FilterBy, FilterTags, LibraryId, PageRequest, PageResult, ReadListId, SeriesId,
};

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::AuthorDto;
use crate::routes::ApiState;

/// Routes of `/api/v1` referential metadata, for the caller to authenticate.
#[deprecated = "Use the GET /api/v2 referential routes instead."]
#[expect(
    deprecated,
    reason = "Registers the deprecated handlers that Kotlin still serves."
)]
pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/authors", get(get_authors))
        .route("/api/v1/authors/names", get(get_author_names))
        .route("/api/v1/authors/roles", get(get_author_roles))
        .route("/api/v1/genres", series_values(SeriesValue::Genre))
        .route(
            "/api/v1/sharing-labels",
            series_values(SeriesValue::SharingLabel),
        )
        .route("/api/v1/languages", series_values(SeriesValue::Language))
        .route("/api/v1/publishers", series_values(SeriesValue::Publisher))
        .route("/api/v1/tags", tags(FilterTags::Both))
        .route("/api/v1/tags/book", tags(FilterTags::Book))
        .route("/api/v1/tags/series", tags(FilterTags::Series))
        .route("/api/v1/age-ratings", get(get_age_ratings))
        .route("/api/v1/series/release-dates", get(get_release_dates))
        .into_router()
}

/// The items of `result`, as `D`.
fn items<T, D: From<T>, E: std::fmt::Display>(
    result: Result<PageResult<T>, E>,
    path: &str,
) -> Result<Json<Vec<D>>, ApiError> {
    let result = result.map_err(|error| ApiError::internal_error(path, error))?;
    Ok(Json(result.items.into_iter().map(D::from).collect()))
}

/// `getAuthorsDeprecated`: one library, else one collection, else one series.
#[deprecated = "Use GET /api/v2/authors instead. Deprecated since 1.20.0."]
async fn get_authors(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<Vec<AuthorDto>>, ApiError> {
    let params = RequestParams::parse(query.as_deref());
    let filter = first_of(&params, &[Owner::Library, Owner::Collection, Owner::Series]);
    let search = params.joined("search");
    let result: Result<PageResult<BookAuthor>, _> = referential
        .authors(
            &user,
            search.as_deref(),
            None,
            filter.as_ref(),
            PageRequest::Unpaged,
        )
        .await;
    items(result, uri.path())
}

#[deprecated = "Use GET /api/v2/authors/names instead. Deprecated since 1.26.0."]
async fn get_author_names(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<Vec<String>>, ApiError> {
    let params = RequestParams::parse(query.as_deref());
    let search = params.joined("search");
    let result = referential
        .credit_names(
            &user,
            Credit::Author,
            search.as_deref(),
            None,
            None,
            PageRequest::Unpaged,
        )
        .await;
    items(result, uri.path())
}

#[deprecated = "Use GET /api/v2/authors/roles instead. Deprecated since 1.26.0."]
async fn get_author_roles(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<String>>, ApiError> {
    let result = referential
        .credit_kinds(&user, Credit::Author, None, PageRequest::Unpaged)
        .await;
    items(result, uri.path())
}

/// `getGenres`, `getSharingLabels`, `getLanguages` and `getPublishers`.
#[deprecated = "Use GET /api/v2/{genres,sharing-labels,languages,publishers} instead. Deprecated since 1.26.0."]
fn series_values(kind: SeriesValue) -> MethodRouter<ApiState> {
    get(
        move |Authenticated(user): Authenticated,
              State(referential): State<ReferentialService>,
              OriginalUri(uri): OriginalUri,
              RawQuery(query): RawQuery| async move {
            let filter = libraries_or_collection(&RequestParams::parse(query.as_deref()));
            let result = referential
                .series_values(&user, kind, None, filter.as_ref(), PageRequest::Unpaged)
                .await;
            items::<_, String, _>(result, uri.path())
        },
    )
}

/// `getTags` (libraries, else one collection), `getBookTags` (one series, else one read list,
/// else libraries) and `getSeriesTags` (one library, else one collection).
#[deprecated = "Use GET /api/v2/tags instead. Deprecated since 1.26.0."]
fn tags(source: FilterTags) -> MethodRouter<ApiState> {
    get(
        move |Authenticated(user): Authenticated,
              State(referential): State<ReferentialService>,
              OriginalUri(uri): OriginalUri,
              RawQuery(query): RawQuery| async move {
            let params = RequestParams::parse(query.as_deref());
            let filter = match source {
                FilterTags::Both => libraries_or_collection(&params),
                FilterTags::Book => first_of(&params, &[Owner::Series, Owner::ReadList])
                    .or_else(|| libraries(&params)),
                FilterTags::Series => first_of(&params, &[Owner::Library, Owner::Collection]),
            };
            let result = referential
                .tags(&user, None, source, filter.as_ref(), PageRequest::Unpaged)
                .await;
            items::<_, String, _>(result, uri.path())
        },
    )
}

/// Unrated series show as `None`.
#[deprecated = "Use GET /api/v2/age-ratings instead. Deprecated since 1.26.0."]
async fn get_age_ratings(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<Vec<String>>, ApiError> {
    let filter = libraries_or_collection(&RequestParams::parse(query.as_deref()));
    let ratings = referential
        .age_ratings_with_unrated(&user, filter.as_ref())
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    let ratings = ratings
        .into_iter()
        .map(|rating| rating.map_or_else(|| "None".to_owned(), |rating| rating.to_string()));
    Ok(Json(ratings.collect()))
}

/// The years of the release dates, latest first.
#[deprecated = "Use GET /api/v2/series/release-years instead. Deprecated since 1.26.0."]
async fn get_release_dates(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<Vec<String>>, ApiError> {
    let filter = libraries_or_collection(&RequestParams::parse(query.as_deref()));
    let result = referential
        .series_release_years(&user, filter.as_ref(), PageRequest::Unpaged)
        .await;
    items(result, uri.path())
}

/// A `Set<String> library_id`, else a `String collection_id`.
fn libraries_or_collection(params: &RequestParams) -> Option<FilterBy> {
    libraries(params).or_else(|| first_of(params, &[Owner::Collection]))
}

fn libraries(params: &RequestParams) -> Option<FilterBy> {
    let ids = params.string_set("library_id");
    (!ids.is_empty()).then(|| FilterBy::Libraries(ids.into_iter().map(LibraryId::from).collect()))
}

/// A single-ID filter parameter.
#[derive(Clone, Copy)]
enum Owner {
    Library,
    Collection,
    Series,
    ReadList,
}

/// The filter of the first of `owners` whose parameter is present; Spring joins repeated values.
fn first_of(params: &RequestParams, owners: &[Owner]) -> Option<FilterBy> {
    owners.iter().find_map(|owner| {
        let name = match owner {
            Owner::Library => "library_id",
            Owner::Collection => "collection_id",
            Owner::Series => "series_id",
            Owner::ReadList => "readlist_id",
        };
        let id = params.joined(name)?;
        Some(match owner {
            Owner::Library => FilterBy::Libraries([LibraryId::from(id)].into()),
            Owner::Collection => FilterBy::Collections([id].into()),
            Owner::Series => FilterBy::Series([SeriesId::from(id)].into()),
            Owner::ReadList => FilterBy::ReadLists([ReadListId::from(id)].into()),
        })
    })
}
