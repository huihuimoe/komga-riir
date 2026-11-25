//! The poster routes `ReadListController` and `SeriesCollectionController` share.

use axum::extract::{OriginalUri, Path, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use komga_service::{ArtworkError, ArtworkService, PosterSet};
use komga_types::{CollectionId, KomgaUser, ReadListId, ThumbnailDetails, ThumbnailId};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::auth::extractors::{Admin, Authenticated};
use crate::error::ApiError;
use crate::mvc::media::jpeg;
use crate::mvc::multipart_form::MultipartForm;
use crate::mvc::produces::{Controller, JPEG, JSON};
use crate::rest::dto::{ThumbnailReadListDto, ThumbnailSeriesCollectionDto};
use crate::routes::ApiState;

/// A read list or collection as its poster routes serve it.
pub(super) trait PosterOwner: PosterSet + DeserializeOwned + 'static {
    type Dto: Serialize + Send;

    fn dto(&self, thumbnail: ThumbnailDetails) -> Self::Dto;

    /// The selected poster, or a mosaic of the members `user` sees.
    fn poster(
        &self,
        artwork: &ArtworkService,
        user: &KomgaUser,
    ) -> impl Future<Output = Result<Vec<u8>, ArtworkError>> + Send;
}

impl PosterOwner for ReadListId {
    type Dto = ThumbnailReadListDto;

    fn dto(&self, thumbnail: ThumbnailDetails) -> Self::Dto {
        ThumbnailReadListDto::new(self, thumbnail)
    }

    async fn poster(
        &self,
        artwork: &ArtworkService,
        user: &KomgaUser,
    ) -> Result<Vec<u8>, ArtworkError> {
        artwork.read_list_thumbnail(user, self).await
    }
}

impl PosterOwner for CollectionId {
    type Dto = ThumbnailSeriesCollectionDto;

    fn dto(&self, thumbnail: ThumbnailDetails) -> Self::Dto {
        ThumbnailSeriesCollectionDto::new(self, thumbnail)
    }

    async fn poster(
        &self,
        artwork: &ArtworkService,
        user: &KomgaUser,
    ) -> Result<Vec<u8>, ArtworkError> {
        artwork.collection_thumbnail(user, self).await
    }
}

/// The poster routes of the sets under `root`.
pub(super) fn routes<S: PosterOwner>(root: &str) -> Router<ApiState> {
    Controller::new(JSON)
        .route_producing(
            &format!("{root}/{{id}}/thumbnail"),
            JPEG,
            get(thumbnail::<S>),
        )
        .route(&format!("{root}/{{id}}/thumbnails"), get(thumbnails::<S>))
        .route_multipart(
            &format!("{root}/{{id}}/thumbnails"),
            post(add_thumbnail::<S>),
        )
        .route_producing(
            &format!("{root}/{{id}}/thumbnails/{{thumbnail_id}}"),
            JPEG,
            get(thumbnail_by_id::<S>),
        )
        .route(
            &format!("{root}/{{id}}/thumbnails/{{thumbnail_id}}"),
            delete(delete_thumbnail::<S>),
        )
        .route(
            &format!("{root}/{{id}}/thumbnails/{{thumbnail_id}}/selected"),
            put(select_thumbnail::<S>),
        )
        .into_router()
}

/// Kept an hour by private caches.
async fn thumbnail<S: PosterOwner>(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<S>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let bytes = id
        .poster(&artwork, &user)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    let mut response = jpeg(bytes);
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static("max-age=3600, private"),
    );
    Ok(response)
}

async fn thumbnails<S: PosterOwner>(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<S>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Vec<S::Dto>>, ApiError> {
    let thumbnails = artwork
        .set_thumbnails(&user, &id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(Json(
        thumbnails
            .into_iter()
            .map(|thumbnail| id.dto(thumbnail))
            .collect(),
    ))
}

async fn thumbnail_by_id<S: PosterOwner>(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path((id, thumbnail_id)): Path<(S, ThumbnailId)>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let bytes = artwork
        .set_thumbnail_by_id(&user, &id, &thumbnail_id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(jpeg(bytes))
}

/// `file` and `selected` bind before `@PreAuthorize` runs.
async fn add_thumbnail<S: PosterOwner>(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<S>,
    OriginalUri(uri): OriginalUri,
    form: MultipartForm,
) -> Result<Json<S::Dto>, ApiError> {
    let path = uri.path();
    let file = form.required_file("file", path)?;
    let selected = form
        .params()
        .boolean_or("selected", true)
        .map_err(|mismatch| ApiError::type_mismatch(&mismatch, path))?;
    if !user.is_admin() {
        return Err(ApiError::forbidden(path));
    }
    let thumbnail = artwork
        .add_set_thumbnail(&user, &id, file, selected)
        .await
        .map_err(|error| ApiError::from_artwork(&error, path))?;
    Ok(Json(id.dto(thumbnail)))
}

async fn select_thumbnail<S: PosterOwner>(
    Admin(user): Admin,
    State(artwork): State<ArtworkService>,
    Path((id, thumbnail_id)): Path<(S, ThumbnailId)>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    artwork
        .select_set_thumbnail(&user, &id, &thumbnail_id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}

async fn delete_thumbnail<S: PosterOwner>(
    Admin(user): Admin,
    State(artwork): State<ArtworkService>,
    Path((id, thumbnail_id)): Path<(S, ThumbnailId)>,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode, ApiError> {
    artwork
        .delete_set_thumbnail(&user, &id, &thumbnail_id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(StatusCode::ACCEPTED)
}
