//! Server information fetched from the Komga website and GitHub.

mod actuator;
mod announcements;
mod releases;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::response::Response;
use axum::routing::get;
use tokio::net::TcpListener;

use crate::support::{TestServer, app_config};

/// Serves `app` locally; returns the URL of its `/feed`.
async fn serve(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Fake site must bind");
    let address = listener.local_addr().expect("Fake site address");
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{address}/feed")
}

/// A local stand-in for the website or GitHub, answering the `n`th request (from 0) with `answer(n)`.
async fn fake_site(answer: fn(usize) -> Response) -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let app = Router::new().route(
        "/feed",
        get(move || async move { answer(counted.fetch_add(1, Ordering::SeqCst)) }),
    );
    (serve(app).await, hits)
}

/// A claimed server fetching both the announcements and the releases from `site`.
async fn server(site: &str, directory: &Path) -> TestServer {
    let mut config = app_config(directory);
    config.announcements_url = site.to_owned();
    config.releases_url = site.to_owned();
    config.server_info_timeout = Duration::from_millis(300);
    TestServer::start_claimed_with(config).await
}
