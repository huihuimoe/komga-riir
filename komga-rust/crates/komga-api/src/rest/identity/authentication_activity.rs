//! `UserController` authentication activity routes.

use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use komga_service::IdentityError;
use komga_types::{ActivitySort, KomgaUser, SortDirection, SortOrder};

use crate::auth::AuthState;
use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::pageable::{SpringPage, keyed, page_request, sort_orders, spring_page};
use crate::mvc::produces::{Controller, JSON};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::AuthenticationActivityDto;
use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Controller::new(JSON)
        .route("/api/v2/users/me/authentication-activity", get(mine))
        .route("/api/v2/users/authentication-activity", get(all))
        .route(
            "/api/v2/users/{id}/authentication-activity/latest",
            get(latest),
        )
        .into_router()
}

async fn mine(
    Authenticated(user): Authenticated,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<AuthenticationActivityDto>>, ApiError> {
    page(&state, Some(&user), uri.path(), query.as_deref()).await
}

async fn all(
    _admin: Admin,
    State(state): State<AuthState>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<SpringPage<AuthenticationActivityDto>>, ApiError> {
    page(&state, None, uri.path(), query.as_deref()).await
}

/// Both lists: newest first unless a sort is requested, `unpaged` to get everything.
async fn page(
    state: &AuthState,
    user: Option<&KomgaUser>,
    path: &str,
    query: Option<&str>,
) -> Result<Json<SpringPage<AuthenticationActivityDto>>, ApiError> {
    let params = RequestParams::parse(query);
    let unpaged = params
        .boolean_or("unpaged", false)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    let page = page_request(&params, unpaged);
    let sort = if sort_orders(&params).is_empty() {
        vec![SortOrder {
            key: ActivitySort::DateTime,
            direction: SortDirection::Desc,
        }]
    } else {
        keyed(&params, sort_key)
    };
    let result = state
        .identity()
        .activities(user, &sort, page)
        .await
        .map_err(|error| ApiError::from_identity(error, path))?;
    Ok(spring_page(result, page, &sort))
}

async fn latest(
    Authenticated(user): Authenticated,
    State(state): State<AuthState>,
    Path(id): Path<String>,
    OriginalUri(uri): OriginalUri,
    RawQuery(query): RawQuery,
) -> Result<Json<AuthenticationActivityDto>, ApiError> {
    let path = uri.path();
    if !user.is_admin() && user.id.as_str() != id {
        return Err(ApiError::forbidden(path));
    }
    let params = RequestParams::parse(query.as_deref());
    let api_key_id = params.first("apikey_id");
    match state.identity().latest_activity(&id, api_key_id).await {
        Ok(Some(activity)) => Ok(Json(activity.into())),
        Ok(None) | Err(IdentityError::UserNotFound) => Err(not_found(path)),
        Err(error) => Err(ApiError::from_identity(error, path)),
    }
}

/// `ResponseStatusException(HttpStatus.NOT_FOUND)`.
fn not_found(path: &str) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path)
}

/// `AuthenticationActivityDao.sorts`.
fn sort_key(property: &str) -> Option<ActivitySort> {
    match property {
        "dateTime" => Some(ActivitySort::DateTime),
        "email" => Some(ActivitySort::Email),
        "success" => Some(ActivitySort::Success),
        "ip" => Some(ActivitySort::Ip),
        "error" => Some(ActivitySort::Error),
        "userId" => Some(ActivitySort::UserId),
        "userAgent" => Some(ActivitySort::UserAgent),
        _ => None,
    }
}
