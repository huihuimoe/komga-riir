use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;

#[tokio::test]
async fn test_http_server_uses_file_database_and_shuts_down_gracefully() {
    let temp_dir =
        komga_test_support::tempdir().expect("Failed to create temporary data directory");
    let config_dir = temp_dir.path().to_path_buf();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral HTTP port");
    let address = listener
        .local_addr()
        .expect("Listener must expose its actual port");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let server = tokio::spawn(komga_rust::app::serve_with_shutdown(
        crate::support::app_config(&config_dir),
        listener,
        async move {
            let _ = shutdown_rx.await;
        },
    ));

    let mut stream = TcpStream::connect(address)
        .await
        .expect("HTTP server must accept TCP connections");
    stream
        .write_all(b"GET /api/v1/claim HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("Failed to send HTTP request");
    let mut response = String::new();
    timeout(Duration::from_secs(5), stream.read_to_string(&mut response))
        .await
        .expect("HTTP server did not complete the response")
        .expect("Failed to read HTTP response");

    let (headers, body) = response
        .split_once("\r\n\r\n")
        .expect("Server must return a valid HTTP response");
    assert!(headers.starts_with("HTTP/1.1 200 OK"), "{headers}");
    assert!(body.contains("\"isClaimed\":false"), "{body}");
    assert!(config_dir.join("database.sqlite").is_file());

    shutdown_tx
        .send(())
        .expect("Server shutdown receiver must remain alive");
    timeout(Duration::from_secs(5), server)
        .await
        .expect("HTTP server did not shut down")
        .expect("HTTP server task panicked")
        .expect("HTTP server failed while shutting down");
    // Closing the last connection checkpoints and removes the write-ahead logs.
    for log in ["database.sqlite-wal", "tasks.sqlite-wal"] {
        assert!(!config_dir.join(log).exists(), "{log} remains");
    }
}
