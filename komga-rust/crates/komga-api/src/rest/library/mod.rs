//! Library, file deletion, page hash, and transient book routes.

mod file_deletion;
mod libraries;
mod page_hash;
mod transient_book;

use axum::Router;

use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Router::new()
        .merge(libraries::routes())
        .merge(file_deletion::routes())
        .merge(page_hash::routes())
        .merge(transient_book::routes())
}
