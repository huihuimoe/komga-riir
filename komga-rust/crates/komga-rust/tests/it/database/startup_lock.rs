use std::future::pending;
use std::time::{Duration, Instant};

use komga_db::DirectoryLock;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[tokio::test]
async fn test_locked_directory_is_rejected_before_database_creation() {
    let temp_dir =
        komga_test_support::tempdir().expect("Failed to create temporary data directory");
    let config_dir = temp_dir.path().to_path_buf();
    let _lock = DirectoryLock::acquire(&config_dir).expect("Initial data directory lock succeeds");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind an ephemeral port");

    let error = komga_rust::app::serve_with_lock_timeout(
        crate::support::app_config(&config_dir),
        listener,
        pending(),
        Duration::from_millis(150),
    )
    .await
    .expect_err("Startup must reject an already-locked data directory");

    assert!(
        error.chain().any(|cause| cause
            .to_string()
            .contains("Directory is locked by another instance")),
        "Unexpected startup error: {error:#}"
    );
    assert!(
        !config_dir.join("database.sqlite").exists(),
        "Rejected startup must not create or migrate the database"
    );
}

#[tokio::test]
async fn test_startup_waits_for_directory_lock_release() {
    let temp_dir =
        komga_test_support::tempdir().expect("Failed to create temporary data directory");
    let config_dir = temp_dir.path().to_path_buf();
    let lock = DirectoryLock::acquire(&config_dir).expect("Initial data directory lock succeeds");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind an ephemeral port");
    let address = listener
        .local_addr()
        .expect("Listener must have a local address");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let server = tokio::spawn(komga_rust::app::serve_with_lock_timeout(
        crate::support::app_config(&config_dir),
        listener,
        async move {
            let _ = shutdown_rx.await;
        },
        Duration::from_secs(5),
    ));

    tokio::time::sleep(Duration::from_millis(150)).await;
    drop(lock);

    let client = reqwest::Client::new();
    let url = format!("http://{address}/actuator/health");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(response) = client.get(&url).send().await
            && response.status().is_success()
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Server did not become healthy after directory lock release"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let _ = shutdown_tx.send(());
    server
        .await
        .expect("Server task must not panic")
        .expect("Server must shut down cleanly");
}

#[tokio::test]
async fn test_startup_aborts_lock_wait_on_shutdown() {
    let temp_dir =
        komga_test_support::tempdir().expect("Failed to create temporary data directory");
    let config_dir = temp_dir.path().to_path_buf();
    let _lock = DirectoryLock::acquire(&config_dir).expect("Initial data directory lock succeeds");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind an ephemeral port");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let started = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = shutdown_tx.send(());
    });

    komga_rust::app::serve_with_lock_timeout(
        crate::support::app_config(&config_dir),
        listener,
        async move {
            let _ = shutdown_rx.await;
        },
        Duration::from_secs(30),
    )
    .await
    .expect("Shutdown while waiting for lock must exit cleanly");

    assert!(
        started.elapsed() < Duration::from_secs(5),
        "Shutdown must abort lock wait immediately instead of waiting for timeout"
    );
    assert!(
        !config_dir.join("database.sqlite").exists(),
        "Aborted startup must not create or migrate the database"
    );
}
