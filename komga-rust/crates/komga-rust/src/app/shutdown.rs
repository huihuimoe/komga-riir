//! Graceful shutdown orchestration for the HTTP server, background workers, and databases.

use std::future::Future;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::connect_info::{ConnectInfo, IntoMakeServiceWithConnectInfo};
use axum::middleware::AddExtension;
use komga_api::{LocalAddress, SseConnections};
use komga_db::Database;
use komga_scheduler::Scheduler;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Spring's default `spring.lifecycle.timeout-per-shutdown-phase`.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

/// What runs beside the HTTP server.
pub(super) struct Workers {
    pub(super) scheduler: Scheduler,
    pub(super) search_index: JoinHandle<()>,
    pub(super) stop_search_index: oneshot::Sender<()>,
    pub(super) periodic_scans: JoinHandle<()>,
    pub(super) evictions: JoinHandle<()>,
    pub(super) activity_cleanup: JoinHandle<()>,
}

impl Workers {
    async fn stop(self) {
        // Before the scheduler stops, so that no scan is queued once it has.
        self.periodic_scans.abort();
        self.evictions.abort();
        self.activity_cleanup.abort();
        self.scheduler.shutdown().await;
        let _ = self.stop_search_index.send(());
        if self.search_index.await.is_err() {
            error!("Search index updater panicked");
        }
    }
}

/// Resolves on the external `shutdown` signal or on `POST /actuator/shutdown`.
pub(super) async fn shutdown_or_requested(
    shutdown: impl Future<Output = ()>,
    requested: CancellationToken,
) {
    tokio::select! {
        () = shutdown => {}
        () = requested.cancelled() => info!("Shutdown requested through the API"),
    }
}

/// Once `shutdown` resolves, stops accepting requests and stops the workers at the same time:
/// the scheduler stops claiming tasks and asks running ones to stop. When both are done, the
/// databases are closed. All of it is bounded by [`shutdown_deadline`].
pub(super) async fn serve_until_shutdown<F>(
    server: axum::serve::Serve<
        TcpListener,
        IntoMakeServiceWithConnectInfo<axum::Router, LocalAddress>,
        AddExtension<axum::Router, ConnectInfo<LocalAddress>>,
    >,
    sse_connections: SseConnections,
    workers: Workers,
    databases: [Database; 2],
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let shutting_down = CancellationToken::new();
    let server = server.with_graceful_shutdown({
        let shutting_down = shutting_down.clone();
        async move {
            shutdown.await;
            shutting_down.cancel();
            sse_connections.close();
        }
    });
    let served = async {
        let served = server.await.context("HTTP server failed");
        shutting_down.cancel();
        served
    };
    let stopped = async {
        shutting_down.cancelled().await;
        workers.stop().await;
    };
    let closed = async {
        let (served, ()) = tokio::join!(served, stopped);
        for database in databases {
            database.close().await;
        }
        served
    };
    tokio::select! {
        served = closed => served,
        () = shutdown_deadline(&shutting_down) => Ok(()),
    }
}

/// Resolves once a shutdown has run for [`SHUTDOWN_TIMEOUT`] or is interrupted by Ctrl-C. The
/// tasks then abandoned are released when the server next starts.
async fn shutdown_deadline(started: &CancellationToken) {
    started.cancelled().await;
    tokio::select! {
        () = tokio::time::sleep(SHUTDOWN_TIMEOUT) => {
            warn!(timeout = ?SHUTDOWN_TIMEOUT, "Shutdown timed out; abandoning open connections and running tasks");
        }
        Ok(()) = tokio::signal::ctrl_c() => {
            warn!("Second interrupt received; abandoning open connections and running tasks");
        }
    }
}

pub(super) async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                error!(error = %error, "Failed to register SIGTERM handler");
                return;
            }
        };

        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    error!(error = %error, "Failed to receive Ctrl-C signal");
                }
            }
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        error!(error = %error, "Failed to receive Ctrl-C signal");
    }

    info!("Shutdown signal received; stopping HTTP server gracefully");
}
