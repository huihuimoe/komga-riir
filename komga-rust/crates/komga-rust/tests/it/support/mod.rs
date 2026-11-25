//! Real HTTP server harness: `komga_rust::app` listening on `127.0.0.1:0` over a file database.

pub mod cbz;
pub mod db;
pub mod openapi;
pub mod posters;
pub mod process;
pub mod search;
pub mod tasks;
pub mod template;

use std::path::Path;
use std::thread;
use std::time::Duration;

use komga_rust::config::{AppConfig, CliArgs, Environment};
use reqwest::header::{COOKIE, SET_COOKIE};
use reqwest::{Client, RequestBuilder};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::timeout;

pub const ADMIN_EMAIL: &str = "admin@example.com";
pub const ADMIN_PASSWORD: &str = "admin12345";

/// Resolves the configuration for `config_dir` with an empty environment. The search index commits
/// each change at once, like Kotlin's test profile.
pub fn app_config(config_dir: &Path) -> AppConfig {
    let args = CliArgs {
        config_dir: Some(config_dir.to_path_buf()),
        ..CliArgs::default()
    };
    let mut config = AppConfig::resolve(&args, &Environment::default())
        .expect("Test configuration must resolve");
    config.lucene.commit_delay.value = Duration::ZERO;
    config
}

pub struct TestServer {
    port: u16,
    base_url: String,
    client: Client,
    /// The admin `KOMGA-SESSION` cookie of a claimed server.
    admin_session: Option<String>,
    /// `None` once stopped.
    server: Option<ServerThread>,
}

/// The server runs on a runtime of its own, so that dropping a [`TestServer`] that was not
/// stopped can wait for it to release the data directory: the test's runtime is busy dropping
/// the test, and the data directory is usually removed right after.
struct ServerThread {
    shutdown: oneshot::Sender<()>,
    thread: thread::JoinHandle<anyhow::Result<()>>,
}

impl ServerThread {
    /// Asks the server to stop and waits until it has.
    fn stop(self) -> thread::Result<anyhow::Result<()>> {
        let _ = self.shutdown.send(());
        self.thread.join()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            let _ = server.stop();
        }
    }
}

impl TestServer {
    pub async fn start(config_dir: &Path) -> Self {
        Self::start_with(app_config(config_dir)).await
    }

    pub async fn start_with(config: AppConfig) -> Self {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("Failed to bind ephemeral HTTP port");
        listener
            .set_nonblocking(true)
            .expect("Failed to make the listener non-blocking");
        let address = listener
            .local_addr()
            .expect("Listener must expose its actual port");
        let (shutdown, shutdown_rx) = oneshot::channel();
        let thread = thread::spawn(move || {
            let runtime = komga_rust::runtime_builder()
                .worker_threads(2)
                .build()
                .expect("Failed to build server runtime");
            runtime.block_on(async move {
                let listener =
                    TcpListener::from_std(listener).expect("Failed to register listener");
                komga_rust::app::serve_with_shutdown(config, listener, async move {
                    let _ = shutdown_rx.await;
                })
                .await
            })
        });
        Self {
            port: address.port(),
            base_url: format!("http://{address}"),
            client: Client::new(),
            admin_session: None,
            server: Some(ServerThread { shutdown, thread }),
        }
    }

    /// Starts a server over a fresh data directory, claims it with the admin account and logs the
    /// admin in, once it has built its empty search index.
    pub async fn start_claimed(config_dir: &Path) -> Self {
        Self::start_claimed_with(app_config(config_dir)).await
    }

    /// [`Self::start_claimed`] with an adjusted configuration.
    pub async fn start_claimed_with(config: AppConfig) -> Self {
        let config_dir = config.config_dir.value.clone();
        let seeded = if can_use_claimed_template(&config) {
            let dir = claimed_template_dir().await;
            template::copy_into(&dir, &config_dir);
            true
        } else {
            false
        };
        let mut server = Self::start_with(config).await;
        if !seeded {
            claim_admin(&server).await;
        }
        let response = server
            .admin(reqwest::Method::GET, "/api/v2/users/me")
            .send()
            .await
            .expect("Admin login failed");
        let cookie = response.headers()[SET_COOKIE]
            .to_str()
            .expect("ASCII cookie");
        server.admin_session = Some(cookie.split(';').next().expect("Cookie pair").to_owned());
        if !seeded {
            // Queued before the server answers on a cold start.
            wait_for_initial_index(&config_dir).await;
        }
        server
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn request(&self, method: reqwest::Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, format!("{}{path}", self.base_url))
    }

    /// An admin request in the admin session once there is one, as a browser sends it. Basic
    /// authentication hashes the password on every request (bcrypt cost 10, tens of milliseconds)
    /// and records an activity; the session skips both, and the credentials still authenticate
    /// after it is revoked.
    pub fn admin(&self, method: reqwest::Method, path: &str) -> RequestBuilder {
        let request = self
            .request(method, path)
            .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD));
        match &self.admin_session {
            Some(cookie) => request.header(COOKIE, cookie),
            None => request,
        }
    }

    /// Stops accepting requests and waits for graceful shutdown, releasing the data directory.
    pub async fn stop(mut self) {
        let server = self.server.take().expect("A server is stopped once");
        timeout(
            Duration::from_secs(5),
            tokio::task::spawn_blocking(|| server.stop()),
        )
        .await
        .expect("HTTP server did not shut down")
        .expect("Failed to wait for the HTTP server")
        .expect("HTTP server thread panicked")
        .expect("HTTP server failed while shutting down");
    }
}

pub async fn json(response: reqwest::Response) -> serde_json::Value {
    response
        .json()
        .await
        .expect("Response body must be valid JSON")
}

/// The template holds a migrated, claimed (pre-login) database pair and an empty default-ngram
/// search index; fall back to cold startup whenever a test customizes those paths or settings.
fn can_use_claimed_template(config: &AppConfig) -> bool {
    let dir = &config.config_dir.value;
    config.context_path.is_none()
        && config.database_file.value == dir.join("database.sqlite")
        && config.tasks_database_file.value == dir.join("tasks.sqlite")
        && config.lucene.data_directory.value == dir.join("lucene")
        && config.lucene.ngram() == komga_search::NgramOptions::default()
        && !config.database_file.value.exists()
        && !config.tasks_database_file.value.exists()
        && !config.lucene.data_directory.value.exists()
}

async fn claim_admin(server: &TestServer) {
    let response = server
        .request(reqwest::Method::POST, "/api/v1/claim")
        .header("X-Komga-Email", ADMIN_EMAIL)
        .header("X-Komga-Password", ADMIN_PASSWORD)
        .send()
        .await
        .expect("Claim request failed");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

async fn wait_for_initial_index(config_dir: &Path) {
    tasks::wait_for_tasks(config_dir, |rows| {
        rows.iter().all(|row| row.0 != "REBUILD_INDEX")
    })
    .await;
}

async fn claimed_template_dir() -> std::path::PathBuf {
    let key = format!(
        "claimed-server-v1-{:08x}",
        template::claimed_server_fingerprint(ADMIN_EMAIL, ADMIN_PASSWORD)
    );
    template::cached_dir(&key, |dir| async move {
        let server = TestServer::start_with(app_config(&dir)).await;
        claim_admin(&server).await;
        wait_for_initial_index(&dir).await;
        server.stop().await;
    })
    .await
}
