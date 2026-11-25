//! Runtime assembly and HTTP server lifecycle.

pub(crate) mod local_filesystem;
mod periodic_scans;
mod services;
mod shutdown;
mod storage;
mod web_ui;

use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV6};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use komga_api::{LocalAddress, WebServerSettings};
use komga_scheduler::{Scheduler, Task, TaskPoolSize, TaskQueue};
use komga_service::{
    FileOperations, FontService, IdentityService, ReadingService, SearchIndexUpdater, TaskHandler,
    TaskHandlerConfig,
};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::{error, info, warn};

use crate::config::AppConfig;
use shutdown::{Workers, serve_until_shutdown, shutdown_or_requested, shutdown_signal};
use storage::{LOCK_WAIT_TIMEOUT, Storage};

/// Initialize storage and serve the HTTP application at the configured address.
pub async fn run(config: AppConfig) -> Result<()> {
    let mut shutdown = Box::pin(shutdown_signal());
    let Some(storage) = Storage::open(&config, LOCK_WAIT_TIMEOUT, &mut shutdown).await? else {
        return Ok(());
    };
    let port = effective_port(config.port.value, storage.settings.server_port)?;
    let listener =
        bind_listener(config.host.as_ref().map(|host| host.value.as_str()), port).await?;

    serve(storage, &config, listener, shutdown).await
}

async fn bind_listener(host: Option<&str>, port: u16) -> Result<TcpListener> {
    if let Some(host) = host {
        return TcpListener::bind((host, port))
            .await
            .with_context(|| format!("Failed to bind HTTP server to {host}:{port}"));
    }
    match bind_dual_stack_v6(port) {
        Ok(listener) => Ok(listener),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            Err(err).with_context(|| format!("Failed to bind HTTP server to [::]:{port}"))
        }
        Err(err) => {
            warn!(
                error = %err,
                port,
                "Failed to bind dual-stack IPv6 listener on [::], falling back to 0.0.0.0"
            );
            TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))
                .await
                .with_context(|| format!("Failed to bind HTTP server to 0.0.0.0:{port}"))
        }
    }
}

/// Bind a dual-stack IPv6 wildcard socket so both IPv6 and IPv4 clients can connect.
fn bind_dual_stack_v6(port: u16) -> io::Result<TcpListener> {
    let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    // Windows defaults IPV6_V6ONLY to true, and Linux hosts may set net.ipv6.bindv6only=1.
    socket.set_only_v6(false)?;
    // Match Tokio's Unix TcpListener default; skip on Windows where SO_REUSEADDR allows hijacking.
    #[cfg(not(windows))]
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0).into())?;
    socket.listen(1024)?;
    TcpListener::from_std(socket.into())
}

/// Serve on an already-bound listener with a caller-provided shutdown signal. The stored server
/// port does not apply because the caller chose the listener.
pub async fn serve_with_shutdown<F>(
    config: AppConfig,
    listener: TcpListener,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    serve_with_lock_timeout(config, listener, shutdown, LOCK_WAIT_TIMEOUT).await
}

/// Like [`serve_with_shutdown`], with an explicit timeout for waiting on the data directory lock.
pub async fn serve_with_lock_timeout<F>(
    config: AppConfig,
    listener: TcpListener,
    shutdown: F,
    lock_wait_timeout: Duration,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let mut shutdown = Box::pin(shutdown);
    let Some(storage) = Storage::open(&config, lock_wait_timeout, &mut shutdown).await? else {
        return Ok(());
    };
    serve(storage, &config, listener, shutdown).await
}

/// `WebServerConfiguration`: a valid stored context path overrides
/// `server.servlet.context-path`.
fn effective_context_path<'path>(
    configured: Option<&'path str>,
    stored: Option<&'path str>,
) -> &'path str {
    match stored {
        Some(path) if komga_types::is_context_path(path) => path,
        Some(path) => {
            warn!(
                context_path = path,
                "Ignoring invalid stored server context path"
            );
            configured.unwrap_or_default()
        }
        None => configured.unwrap_or_default(),
    }
}

/// `WebServerConfiguration`: a stored port overrides `server.port`, but values up to 1 are
/// ignored.
fn effective_port(configured: u16, stored: Option<i32>) -> Result<u16> {
    match stored {
        Some(port) if port > 1 => match u16::try_from(port) {
            Ok(port) => Ok(port),
            Err(_) => bail!("Invalid SERVER_SETTINGS.SERVER_PORT value: {port}"),
        },
        Some(port) => {
            warn!(port, "Ignoring invalid stored server port");
            Ok(configured)
        }
        None => Ok(configured),
    }
}

async fn serve<F>(
    storage: Storage,
    config: &AppConfig,
    listener: TcpListener,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let local_address = listener
        .local_addr()
        .context("Failed to determine HTTP server address")?;
    let configured_context_path = config.context_path.as_ref().map(|path| path.value.as_str());
    let context_path = effective_context_path(
        configured_context_path,
        storage.settings.server_context_path.as_deref(),
    );
    let web_server = WebServerSettings {
        configured_port: config.port.value,
        effective_port: local_address.port(),
        configured_context_path: configured_context_path.map(str::to_owned),
        context_path: context_path.into(),
        trusted_proxies: config.trusted_proxies.value.clone(),
    };
    let tasks = TaskQueue::new(storage.tasks.clone());
    let task_pool_size = TaskPoolSize::new(storage.settings.task_pool_size);
    let reading = ReadingService::new(storage.database.clone());
    let fonts_directory = config.fonts_data_directory.value.clone();
    let fonts = tokio::task::spawn_blocking(move || FontService::load(&fonts_directory))
        .await
        .context("Failed to list fonts")?;
    let api = services::api_services(
        &storage,
        &tasks,
        task_pool_size.clone(),
        web_server,
        reading.clone(),
        fonts,
        config,
    )
    .await?;
    let sse_connections = api.sse_connections.clone();
    let shutdown_requested = api.server_control.shutdown.clone();
    let identity = api.identity.clone();
    let libraries = api.libraries.clone();
    let app = komga_api::router(api);
    let search_index = SearchIndexUpdater::new(storage.database.clone(), storage.search_index);
    // Kotlin `SearchIndexController`.
    if let Some(reason) = search_index.rebuild_reason() {
        info!(%reason, "Queuing a search index rebuild");
        tasks
            .submit(&Task::rebuild_index().highest())
            .await
            .context("Failed to queue the search index rebuild")?;
    }
    let handler = TaskHandler::new(
        storage.database.clone(),
        tasks.clone(),
        TaskHandlerConfig {
            epub_letter_count_threshold: config.epub_divina_letter_count_threshold.value,
            page_hashing: config.page_hashing.value,
        },
        reading.clone(),
        storage.search.clone(),
        storage.events.clone(),
        search_index.clone(),
    );
    let (stop_search_index, search_index_stopped) = oneshot::channel();
    let search_index = search_index
        .start(storage.search, config.lucene.commit_delay.value, async {
            let _ = search_index_stopped.await;
        })
        .await;
    let search_index = tokio::spawn(search_index);
    // Before the tasks that would begin these operations again are resumed.
    FileOperations::new(storage.database.clone())
        .recover()
        .await;
    // Tasks staged by changes committed just before the previous process stopped.
    tasks.hand_over(&storage.database).await;
    let scheduler = Scheduler::start(tasks, handler, &task_pool_size)
        .await
        .context("Failed to recover the task queue")?;
    let workers = Workers {
        scheduler,
        search_index,
        stop_search_index,
        periodic_scans: periodic_scans::spawn(libraries, &storage.events),
        evictions: tokio::spawn(evict_idle_state(reading, identity.clone())),
        activity_cleanup: tokio::spawn(delete_old_activity(identity)),
    };
    info!(address = %local_address, "HTTP server listening");
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<LocalAddress>(),
    );
    let databases = [storage.database, storage.tasks];
    let shutdown = shutdown_or_requested(shutdown, shutdown_requested);
    serve_until_shutdown(server, sse_connections, workers, databases, shutdown).await
}

/// How often idle cached files and expired sessions are pruned.
const EVICTION_PERIOD: Duration = Duration::from_secs(60);

async fn evict_idle_state(reading: ReadingService, identity: IdentityService) {
    let mut ticks = tokio::time::interval(EVICTION_PERIOD);
    loop {
        ticks.tick().await;
        reading.evict_idle_files();
        identity.evict_expired_sessions();
    }
}

/// `AuthenticationActivityCleanupController`: at startup, then daily.
const ACTIVITY_CLEANUP_PERIOD: Duration = Duration::from_secs(24 * 60 * 60);

async fn delete_old_activity(identity: IdentityService) {
    let mut ticks = tokio::time::interval(ACTIVITY_CLEANUP_PERIOD);
    loop {
        ticks.tick().await;
        if let Err(error) = identity.delete_old_activity().await {
            error!(error = %error, "Failed to remove old authentication activity");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use tokio::net::TcpStream;

    use super::{bind_listener, effective_port};

    #[test]
    fn stored_port_overrides_configured_port_unless_invalid() {
        assert_eq!(effective_port(25600, None).expect("no stored port"), 25600);
        assert_eq!(
            effective_port(25600, Some(8080)).expect("stored port"),
            8080
        );
        assert_eq!(effective_port(25600, Some(1)).expect("ignored port"), 25600);
        assert_eq!(
            effective_port(25600, Some(-5)).expect("ignored port"),
            25600
        );
        assert!(effective_port(25600, Some(70000)).is_err());
    }

    #[tokio::test]
    async fn default_listener_accepts_ipv4_and_when_v6_accepts_ipv6() {
        let listener = bind_listener(None, 0)
            .await
            .expect("default wildcard listener must bind");
        let local = listener.local_addr().expect("listener address");
        let port = local.port();

        let (v4_client, v4_server) = tokio::join!(
            TcpStream::connect((Ipv4Addr::LOCALHOST, port)),
            listener.accept()
        );
        v4_client.expect("IPv4 loopback must reach default listener");
        v4_server.expect("listener must accept IPv4 connection");

        if local.is_ipv6() {
            let (v6_client, v6_server) = tokio::join!(
                TcpStream::connect((Ipv6Addr::LOCALHOST, port)),
                listener.accept()
            );
            v6_client.expect("IPv6 loopback must reach dual-stack listener");
            v6_server.expect("listener must accept IPv6 connection");
        }
    }
}
