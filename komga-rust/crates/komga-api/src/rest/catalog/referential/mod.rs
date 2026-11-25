//! Referential metadata routes (`ReferentialV2Controller`, `ReferentialV3Controller`).

mod v1;

use axum::extract::{OriginalUri, RawQuery, State};
use axum::http::{StatusCode, Uri};
use axum::routing::{MethodRouter, get};
use axum::{Json, Router};
use komga_service::{Credit, ReferentialService, SeriesValue};
use komga_types::{FilterBy, FilterTags, LibraryId, PageRequest, PageResult, ReadListId, SeriesId};

use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::page_response::PageResponse;
use crate::mvc::pageable::{SpringPage, page_request};
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::request_params::{RequestParams, TypeMismatch};
use crate::rest::dto::{AuthorDto, NotableEntityDto};
use crate::routes::ApiState;

/// Create routes for `/api/v1`, `/api/v2`, and `/api/v3` referential metadata.
pub(super) fn routes() -> Router<ApiState> {
    #[expect(deprecated, reason = "Kotlin still serves the v1 referential routes.")]
    let v1 = v1::routes();

    Controller::new(JSON)
        .route("/api/v2/authors", get(get_authors))
        .route("/api/v2/authors/names", get(get_author_names))
        .route("/api/v2/authors/roles", get(get_author_roles))
        .route("/api/v2/tags", get(get_tags))
        .route("/api/v2/genres", series_values(SeriesValue::Genre))
        .route(
            "/api/v2/sharing-labels",
            series_values(SeriesValue::SharingLabel),
        )
        .route("/api/v2/languages", series_values(SeriesValue::Language))
        .route("/api/v2/publishers", series_values(SeriesValue::Publisher))
        .route("/api/v2/age-ratings", get(get_age_ratings))
        .route(
            "/api/v2/series/release-years",
            get(get_series_release_years),
        )
        .route("/api/v3/notable-entities", get(get_notable_entities))
        .route(
            "/api/v3/notable-entities/kinds",
            get(get_notable_entity_kinds),
        )
        .route(
            "/api/v3/notable-entities/names",
            get(get_notable_entity_names),
        )
        .into_router()
        .merge(v1)
}

/// The owners a route narrows its values by; routes ignore the parameters of the others.
#[derive(Clone, Copy)]
enum Owners {
    LibrariesAndCollections,
    All,
}

/// A referential request: its parameters, page and filter.
struct Request {
    params: RequestParams,
    page: PageRequest,
    filter: Option<FilterBy>,
    path: String,
}

impl Request {
    fn parse(uri: &Uri, query: Option<&str>, owners: Owners) -> Result<Self, ApiError> {
        Self::try_parse(uri, query, owners)
            .map_err(|mismatch| ApiError::type_mismatch(&mismatch, uri.path()))
    }

    /// Like [`Self::parse`]; `ApiV3ErrorHandlingControllerAdvice` replies with a
    /// `ProblemDetail`.
    fn parse_v3(uri: &Uri, query: Option<&str>) -> Result<Self, ApiError> {
        Self::try_parse(uri, query, Owners::All).map_err(|mismatch| {
            let detail = format!("Invalid value for parameter '{}'", mismatch.name);
            ApiError::problem(StatusCode::BAD_REQUEST, detail, uri.path())
        })
    }

    fn try_parse(uri: &Uri, query: Option<&str>, owners: Owners) -> Result<Self, TypeMismatch> {
        let params = RequestParams::parse(query);
        let unpaged = params.boolean_or("unpaged", false)?;
        let page = page_request(&params, unpaged);
        let filter = filter_by(&params, owners);
        Ok(Self {
            params,
            page,
            filter,
            path: uri.path().to_owned(),
        })
    }

    /// The page of `result`, whose query sorts by a fixed order.
    fn respond<T, D: From<T>, E: std::fmt::Display>(
        &self,
        result: Result<PageResult<T>, E>,
    ) -> Result<Json<SpringPage<D>>, ApiError> {
        let result = result.map_err(|error| ApiError::internal_error(&self.path, error))?;
        let result = PageResult {
            items: result.items.into_iter().map(D::from).collect(),
            total_elements: result.total_elements,
        };
        Ok(Json(SpringPage::new(
            result, self.page, /* sorted= */ true,
        )))
    }

    /// The v3 page of `result`.
    fn respond_v3<T, D: From<T>, E: std::fmt::Display>(
        &self,
        result: Result<PageResult<T>, E>,
    ) -> Result<Json<PageResponse<D>>, ApiError> {
        let result = result.map_err(|error| ApiError::internal_error(&self.path, error))?;
        Ok(Json(PageResponse::new(result, self.page)))
    }
}

async fn get_authors(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<AuthorDto>>, ApiError> {
    let request = Request::parse(&uri, query.as_deref(), Owners::All)?;
    let search = request.params.joined("search");
    let role = request.params.joined("role");
    let filter = request.filter.as_ref();
    let result = referential
        .authors(
            &user,
            search.as_deref(),
            role.as_deref(),
            filter,
            request.page,
        )
        .await;
    request.respond(result)
}

async fn get_author_names(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<String>>, ApiError> {
    let request = Request::parse(&uri, query.as_deref(), Owners::All)?;
    let search = request.params.joined("search");
    let role = request.params.joined("role");
    let filter = request.filter.as_ref();
    let result = referential
        .credit_names(
            &user,
            Credit::Author,
            search.as_deref(),
            role.as_deref(),
            filter,
            request.page,
        )
        .await;
    request.respond(result)
}

async fn get_author_roles(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<String>>, ApiError> {
    let request = Request::parse(&uri, query.as_deref(), Owners::All)?;
    let result = referential
        .credit_kinds(&user, Credit::Author, request.filter.as_ref(), request.page)
        .await;
    request.respond(result)
}

async fn get_tags(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<String>>, ApiError> {
    let request = Request::parse(&uri, query.as_deref(), Owners::All)?;
    let source = match request.params.joined("include").as_deref() {
        None | Some("BOTH") => FilterTags::Both,
        Some("SERIES") => FilterTags::Series,
        Some("BOOK") => FilterTags::Book,
        Some(other) => {
            let message = format!("Invalid value for parameter 'include': {other}");
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                message,
                &request.path,
            ));
        }
    };
    let search = request.params.joined("search");
    let filter = request.filter.as_ref();
    let result = referential
        .tags(&user, search.as_deref(), source, filter, request.page)
        .await;
    request.respond(result)
}

/// `getGenres`, `getSharingLabels`, `getLanguages` and `getPublishers`, which differ only in the
/// listed value.
fn series_values(kind: SeriesValue) -> MethodRouter<ApiState> {
    get(
        move |Authenticated(user): Authenticated,
              State(referential): State<ReferentialService>,
              OriginalUri(uri): OriginalUri,
              RawQuery(query): RawQuery| async move {
            let request = Request::parse(&uri, query.as_deref(), Owners::LibrariesAndCollections)?;
            let search = request.params.joined("search");
            let filter = request.filter.as_ref();
            let result = referential
                .series_values(&user, kind, search.as_deref(), filter, request.page)
                .await;
            request.respond::<_, String, _>(result)
        },
    )
}

async fn get_age_ratings(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<i32>>, ApiError> {
    let request = Request::parse(&uri, query.as_deref(), Owners::LibrariesAndCollections)?;
    let result = referential
        .age_ratings(&user, request.filter.as_ref(), request.page)
        .await;
    request.respond(result)
}

async fn get_series_release_years(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<String>>, ApiError> {
    let request = Request::parse(&uri, query.as_deref(), Owners::LibrariesAndCollections)?;
    let result = referential
        .series_release_years(&user, request.filter.as_ref(), request.page)
        .await;
    request.respond(result)
}

async fn get_notable_entities(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<PageResponse<NotableEntityDto>>, ApiError> {
    let request = Request::parse_v3(&uri, query.as_deref())?;
    let search = request.params.joined("search");
    let kind = request.params.joined("kind");
    let filter = request.filter.as_ref();
    let result = referential
        .notable_entities(
            &user,
            search.as_deref(),
            kind.as_deref(),
            filter,
            request.page,
        )
        .await;
    request.respond_v3(result)
}

async fn get_notable_entity_kinds(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<PageResponse<String>>, ApiError> {
    let request = Request::parse_v3(&uri, query.as_deref())?;
    let filter = request.filter.as_ref();
    let result = referential
        .credit_kinds(&user, Credit::NotableEntity, filter, request.page)
        .await;
    request.respond_v3(result)
}

async fn get_notable_entity_names(
    Authenticated(user): Authenticated,
    State(referential): State<ReferentialService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<PageResponse<String>>, ApiError> {
    let request = Request::parse_v3(&uri, query.as_deref())?;
    let search = request.params.joined("search");
    let kind = request.params.joined("kind");
    let filter = request.filter.as_ref();
    let credit = Credit::NotableEntity;
    let result = referential
        .credit_names(
            &user,
            credit,
            search.as_deref(),
            kind.as_deref(),
            filter,
            request.page,
        )
        .await;
    request.respond_v3(result)
}

/// Kotlin `unpackFilters`: libraries win over collections, then series, then read lists.
fn filter_by(params: &RequestParams, owners: Owners) -> Option<FilterBy> {
    let library_ids = params.string_set("library_id");
    let collection_ids = params.string_set("collection_id");
    if !library_ids.is_empty() {
        return Some(FilterBy::Libraries(
            library_ids.into_iter().map(LibraryId::from).collect(),
        ));
    }
    if !collection_ids.is_empty() {
        return Some(FilterBy::Collections(collection_ids));
    }
    if let Owners::LibrariesAndCollections = owners {
        return None;
    }
    let series_ids = params.string_set("series_id");
    let read_list_ids = params.string_set("readlist_id");
    if !series_ids.is_empty() {
        Some(FilterBy::Series(
            series_ids.into_iter().map(SeriesId::from).collect(),
        ))
    } else if !read_list_ids.is_empty() {
        Some(FilterBy::ReadLists(
            read_list_ids.into_iter().map(ReadListId::from).collect(),
        ))
    } else {
        None
    }
}
