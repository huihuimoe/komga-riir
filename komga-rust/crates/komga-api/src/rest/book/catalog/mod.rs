//! The book lists of `BookController` and of `ReadListController`.

mod deprecated;
mod filters;

use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use komga_service::{CatalogService, Sibling};
use komga_types::{
    BookCondition, BookId, BookSort, LibraryId, ReadListId, SortDirection, SortOrder,
};

use self::filters::{ListFilter, any_of_filters, book_page, book_sort, known_sort};
use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::json_body::JsonBody;
use crate::mvc::pageable::{SpringPage, full_text_sort, page_request, sort_orders};
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::{BookDto, BookSearchDto};
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    #[expect(deprecated, reason = "Kotlin still serves the GET book lists.")]
    let (books, series_books) = (
        get(deprecated::books_deprecated),
        get(deprecated::series_books_deprecated),
    );
    Controller::new(JSON)
        .route("/api/v1/books", books)
        .route("/api/v1/series/{series_id}/books", series_books)
        .route("/api/v1/books/list", post(list))
        .route("/api/v1/books/latest", get(latest))
        .route("/api/v1/books/ondeck", get(on_deck))
        .route("/api/v1/books/duplicates", get(duplicates))
        .route("/api/v1/books/{book_id}", get(book))
        .route(
            "/api/v1/books/{book_id}/previous",
            get(|user, catalog, id, uri| sibling(user, catalog, id, uri, Sibling::Previous)),
        )
        .route(
            "/api/v1/books/{book_id}/next",
            get(|user, catalog, id, uri| sibling(user, catalog, id, uri, Sibling::Next)),
        )
        .route("/api/v1/readlists/{id}/books", get(read_list_books))
        .route(
            "/api/v1/readlists/{id}/books/{book_id}/previous",
            get(|user, catalog, ids, uri| {
                read_list_sibling(user, catalog, ids, uri, Sibling::Previous)
            }),
        )
        .route(
            "/api/v1/readlists/{id}/books/{book_id}/next",
            get(|user, catalog, ids, uri| {
                read_list_sibling(user, catalog, ids, uri, Sibling::Next)
            }),
        )
        .into_router()
}

/// `getBooksLatest`: the most recently modified books first; sort parameters are ignored.
async fn latest(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<BookDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let unpaged = params
        .boolean_or("unpaged", false)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let page = page_request(&params, unpaged);
    let sort = [SortOrder {
        key: BookSort::LastModifiedDate,
        direction: SortDirection::Desc,
    }];
    let result = catalog
        .books(&user, None, None, &sort, page)
        .await
        .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(book_page(
        result,
        page,
        /* sorted= */ true,
        user.is_admin(),
    ))
}

/// `getBooksDuplicates`: by file hash unless sorted by parameters, of which only known
/// properties apply.
async fn duplicates(
    Admin(user): Admin,
    State(catalog): State<CatalogService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<BookDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let unpaged = params
        .boolean_or("unpaged", false)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let page = page_request(&params, unpaged);
    let sort = if sort_orders(&params).is_empty() {
        vec![SortOrder {
            key: BookSort::FileHash,
            direction: SortDirection::Asc,
        }]
    } else {
        known_sort(&params)
    };
    let result = catalog
        .duplicate_books(&user, &sort, page)
        .await
        .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(book_page(result, page, !sort.is_empty(), user.is_admin()))
}

/// `getBooksOnDeck`: paged without sort, in the order of the series' latest reads.
async fn on_deck(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<BookDto>>, ApiError> {
    let params = RequestParams::parse(query.as_deref());
    let page = page_request(&params, /* unpaged= */ false);
    let libraries: Option<Vec<LibraryId>> = params
        .list::<String>("library_id")
        .expect("Every value parses as a string")
        .map(|ids| ids.into_iter().map(LibraryId::from).collect());
    let result = catalog
        .books_on_deck(&user, libraries.as_deref(), page)
        .await
        .map_err(|error| ApiError::from_catalog(&error, uri.path()))?;
    Ok(book_page(
        result,
        page,
        /* sorted= */ false,
        user.is_admin(),
    ))
}

/// `getBooks`: without a requested sort, by relevance when searching and unsorted otherwise;
/// read list numbers only order books when the condition selects a read list.
async fn list(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
    JsonBody(search): JsonBody<BookSearchDto>,
) -> Result<Json<SpringPage<BookDto>>, ApiError> {
    let path = uri.path();
    let full_text = search
        .full_text_search
        .as_deref()
        .filter(|text| !text.trim().is_empty());
    let params = RequestParams::parse(query.as_deref());
    let unpaged = params
        .boolean_or("unpaged", false)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let page = page_request(&params, unpaged);
    let condition = search.condition.as_ref();
    let in_read_list = condition.is_some_and(|condition| !condition.read_lists().is_empty());
    let mut sort = full_text_sort(
        &params,
        book_sort,
        BookSort::Relevance,
        full_text.is_some(),
        &[],
    );
    sort.retain(|order| order.key != BookSort::ReadListNumber || in_read_list);
    let result = catalog
        .books(&user, condition, full_text, &sort, page)
        .await
        .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(book_page(result, page, !sort.is_empty(), user.is_admin()))
}

async fn book(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    Path(book_id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<BookDto>, ApiError> {
    let book = catalog
        .book(&user, &book_id)
        .await
        .map_err(|error| ApiError::from_catalog(&error, uri.path()))?;
    Ok(Json(BookDto::new(book, user.is_admin())))
}

async fn sibling(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    Path(book_id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
    sibling: Sibling,
) -> Result<Json<BookDto>, ApiError> {
    let book = catalog
        .book_sibling(&user, &book_id, sibling)
        .await
        .map_err(|error| ApiError::from_catalog(&error, uri.path()))?;
    Ok(Json(BookDto::new(book, user.is_admin())))
}

/// `getBooksByReadListId`: paged without sort; each filter parameter matches any of its values.
async fn read_list_books(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    Path(id): Path<ReadListId>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<BookDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let mismatch = |mismatch| ApiError::type_mismatch(&mismatch, path);
    let unpaged = params.boolean_or("unpaged", false).map_err(mismatch)?;
    let deleted = params.boolean("deleted").map_err(mismatch)?;
    let page = page_request(&params, unpaged);
    let filters = [
        ListFilter::Library,
        ListFilter::ReadStatus,
        ListFilter::MediaStatus,
        ListFilter::Tag,
        ListFilter::Author,
    ];
    let mut filters = any_of_filters(&params, &filters)
        .map_err(|message| ApiError::new(StatusCode::BAD_REQUEST, message, path))?;
    filters.extend(deleted.map(|deleted| BookCondition::Deleted(deleted.into())));
    let result = catalog
        .read_list_books(&user, &id, filters, page)
        .await
        .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(book_page(
        result,
        page,
        /* sorted= */ true,
        user.is_admin(),
    ))
}

async fn read_list_sibling(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    Path((id, book_id)): Path<(ReadListId, BookId)>,
    OriginalUri(uri): OriginalUri,
    sibling: Sibling,
) -> Result<Json<BookDto>, ApiError> {
    let book = catalog
        .read_list_book_sibling(&user, &id, &book_id, sibling)
        .await
        .map_err(|error| ApiError::from_catalog(&error, uri.path()))?;
    Ok(Json(BookDto::new(book, user.is_admin())))
}
