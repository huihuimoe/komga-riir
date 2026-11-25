//! Book analysis and reading over real HTTP against real book files.

mod analysis;
mod book_poster_upload;
pub(crate) mod book_posters;
mod conversion;
mod details;
mod divina_manifests;
mod duplicate_pages;
mod duplicates;
pub(crate) mod epub;
mod epub_manifest;
mod epub_positions;
mod epub_resources;
mod import;
mod import_recovery;
pub(crate) mod latest;
mod list;
mod list_deprecated;
mod local_artwork;
mod mobi;
mod on_deck;
mod page_hashing;
mod page_removal;
mod pdf;
mod progression;
mod rar;
pub(crate) mod read_progress;
pub(crate) mod reading;
mod thumbnail_regeneration;
pub(crate) mod thumbnails;
mod transient;

use std::fs;
use std::path::Path;

use komga_rust::config::AppConfig;
use reqwest::{Method, StatusCode};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::libraries::create;
use crate::support::tasks::wait_for_scan_work;
use crate::support::{TestServer, app_config, db};

pub(crate) const PNG: &str = "komga-image/tests/fixtures/photo.png";
pub(crate) const GIF: &str = "komga-image/tests/fixtures/animated.gif";
pub(crate) const JPEG: &str = "komga-archive/tests/fixtures/media_types/page.jpg";
const COMIC_INFO: &str = "komga-archive/tests/fixtures/media_types/ComicInfo.xml";

/// A claimed server whose library `comics` holds `books` (path under the root, content), after
/// the creation scan and the analyses it queued.
pub(crate) async fn library_with_books(
    directory: &Path,
    books: &[(&str, Vec<u8>)],
) -> (TestServer, Value, SqlitePool) {
    library_with_books_configured(app_config(directory), books).await
}

/// [`library_with_books`] with an adjusted configuration.
pub(crate) async fn library_with_books_configured(
    config: AppConfig,
    books: &[(&str, Vec<u8>)],
) -> (TestServer, Value, SqlitePool) {
    let directory = config.config_dir.value.clone();
    let directory = directory.as_path();
    let server = TestServer::start_claimed_with(config).await;
    let root = directory.join("comics");
    for (path, content) in books {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("Book directory")).expect("Book directory");
        fs::write(path, content).expect("Book file");
    }
    let library = create(&server, "Comics", &root).await;
    wait_for_scan_work(directory).await;
    let pool = db::main_pool(directory).await;
    (server, library, pool)
}

async fn analyze(server: &TestServer, book_id: &str) {
    let response = server
        .admin(Method::POST, &format!("/api/v1/books/{book_id}/analyze"))
        .send()
        .await
        .expect("Analyze request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}
