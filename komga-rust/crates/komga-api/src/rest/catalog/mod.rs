//! Collections, read lists, shared poster routes, and referential metadata routes.

mod collection;
mod read_list;
mod referential;
mod set_poster;

use axum::Router;

use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Router::new()
        .merge(collection::routes())
        .merge(read_list::routes())
        .merge(referential::routes())
}
