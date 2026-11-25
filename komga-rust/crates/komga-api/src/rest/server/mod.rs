//! Server administration, settings, fonts, filesystem, history, tasks, and announcements.

mod announcement;
mod client_settings;
mod filesystem;
mod fonts;
mod history;
mod release;
pub(crate) mod settings;
mod sync_point;
mod task;

use axum::Router;

use crate::routes::ApiState;

pub(super) fn routes() -> Router<ApiState> {
    Router::new()
        .merge(announcement::routes())
        .merge(client_settings::routes())
        .merge(filesystem::routes())
        .merge(fonts::routes())
        .merge(history::routes())
        .merge(release::routes())
        .merge(settings::routes())
        .merge(sync_point::routes())
        .merge(task::routes())
}

pub(super) fn public_routes() -> Router<ApiState> {
    Router::new()
        .merge(client_settings::public_routes())
        .merge(fonts::public_routes())
}
