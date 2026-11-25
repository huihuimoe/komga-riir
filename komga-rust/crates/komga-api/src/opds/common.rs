//! Kotlin `OpdsCommonController`.

use axum::Router;
use axum::extract::{OriginalUri, Path, State};
use axum::response::Response;
use axum::routing::get;
use komga_service::ArtworkService;
use komga_types::BookId;

use super::OpdsState;
use crate::auth::extractors::Authenticated;
use crate::error::ApiError;
use crate::mvc::media::jpeg;
use crate::mvc::produces::{Controller, JPEG};

pub(super) fn routes() -> Router<OpdsState> {
    Controller::new(JPEG)
        .route("/opds/v1.2/books/{book_id}/thumbnail", get(thumbnail))
        .route("/opds/v2/books/{book_id}/thumbnail", get(thumbnail))
        .into_router()
}

/// The cover at its own size, as a JPEG.
async fn thumbnail(
    Authenticated(user): Authenticated,
    State(artwork): State<ArtworkService>,
    Path(id): Path<BookId>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let bytes = artwork
        .book_poster(&user, &id)
        .await
        .map_err(|error| ApiError::from_artwork(&error, uri.path()))?;
    Ok(jpeg(bytes))
}
