//! Deprecated GET book-list endpoints preserved for Kotlin API parity.

use axum::Json;
use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::StatusCode;
use jiff::tz::TimeZone;
use komga_service::CatalogService;
use komga_types::{BookCondition, BookSort, DateOperator, SeriesId, SortDirection, SortOrder};

use super::filters::{ListFilter, any_of_filters, book_page, book_sort, known_sort};
use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::pageable::{SpringPage, full_text_sort, page_request, sort_orders};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::BookDto;

/// `getAllBooksDeprecated`: without a requested sort, by relevance when searching and unsorted
/// otherwise; each filter parameter matches any of its values, and `released_after` books
/// released from that day on.
#[deprecated = "Use POST /api/v1/books/list instead. Deprecated since 1.19.0."]
pub(super) async fn books_deprecated(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<BookDto>>, ApiError> {
    let path = uri.path();
    let params = RequestParams::parse(query.as_deref());
    let full_text = params.full_text_search();
    let mismatch = |mismatch| ApiError::type_mismatch(&mismatch, path);
    let unpaged = params.boolean_or("unpaged", false).map_err(mismatch)?;
    let released_after = params.date("released_after").map_err(mismatch)?;
    let page = page_request(&params, unpaged);
    let filters = [
        ListFilter::Library,
        ListFilter::MediaStatus,
        ListFilter::ReadStatus,
        ListFilter::Tag,
    ];
    let mut filters = any_of_filters(&params, &filters)
        .map_err(|message| ApiError::new(StatusCode::BAD_REQUEST, message, path))?;
    filters.extend(released_after.map(|date| {
        let date_time = date
            .to_zoned_datetime(TimeZone::UTC)
            .expect("Midnight UTC of a date is a valid instant")
            .timestamp();
        BookCondition::ReleaseDate(DateOperator::After { date_time })
    }));
    let mut sort = full_text_sort(
        &params,
        book_sort,
        BookSort::Relevance,
        full_text.is_some(),
        &[],
    );
    sort.retain(|order| order.key != BookSort::ReadListNumber);
    let condition = BookCondition::AllOf(filters);
    let result = catalog
        .books(&user, Some(&condition), full_text.as_deref(), &sort, page)
        .await
        .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(book_page(result, page, !sort.is_empty(), user.is_admin()))
}

/// `getBooksBySeriesId`: by number unless sorted by parameters; each filter parameter matches any
/// of its values.
#[deprecated = "Use POST /api/v1/books/list instead. Deprecated since 1.19.0."]
pub(super) async fn series_books_deprecated(
    Authenticated(user): Authenticated,
    State(catalog): State<CatalogService>,
    Path(series_id): Path<SeriesId>,
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
        ListFilter::MediaStatus,
        ListFilter::ReadStatus,
        ListFilter::Tag,
        ListFilter::Author,
    ];
    let mut filters = any_of_filters(&params, &filters)
        .map_err(|message| ApiError::new(StatusCode::BAD_REQUEST, message, path))?;
    filters.extend(deleted.map(|deleted| BookCondition::Deleted(deleted.into())));
    let sort = if sort_orders(&params).is_empty() {
        vec![SortOrder {
            key: BookSort::NumberSort,
            direction: SortDirection::Asc,
        }]
    } else {
        known_sort(&params)
    };
    let result = catalog
        .series_books(&user, &series_id, filters, &sort, page)
        .await
        .map_err(|error| ApiError::from_catalog(&error, path))?;
    Ok(book_page(result, page, !sort.is_empty(), user.is_admin()))
}
