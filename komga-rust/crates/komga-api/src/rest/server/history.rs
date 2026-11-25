//! `HistoricalEventController`.

use axum::extract::{OriginalUri, RawQuery, State};
use axum::routing::get;
use axum::{Json, Router};
use komga_service::HistoryService;
use komga_types::{HistoricalEventSort, SortDirection, SortOrder};

use crate::auth::extractors::Admin;
use crate::error::ApiError;
use crate::mvc::pageable::{SpringPage, keyed, page_request, sort_orders, spring_page};
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::HistoricalEventDto;
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v1/history", get(list))
        .into_router()
}

/// Always paged, newest first unless a sort is requested.
async fn list(
    _admin: Admin,
    State(history): State<HistoryService>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<HistoricalEventDto>>, ApiError> {
    let params = RequestParams::parse(query.as_deref());
    let page = page_request(&params, false);
    let sort = if sort_orders(&params).is_empty() {
        vec![SortOrder {
            key: HistoricalEventSort::Timestamp,
            direction: SortDirection::Desc,
        }]
    } else {
        keyed(&params, sort_key)
    };
    let result = history
        .events(&sort, page)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    Ok(spring_page(result, page, &sort))
}

/// `HistoricalEventDtoDao.sorts`.
fn sort_key(property: &str) -> Option<HistoricalEventSort> {
    match property {
        "type" => Some(HistoricalEventSort::Type),
        "bookId" => Some(HistoricalEventSort::BookId),
        "seriesId" => Some(HistoricalEventSort::SeriesId),
        "timestamp" => Some(HistoricalEventSort::Timestamp),
        _ => None,
    }
}
