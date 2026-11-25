//! `/actuator/health`, `/actuator/info`, `/actuator/metrics`, `/actuator/prometheus`,
//! `/actuator/logfile` and `/actuator/shutdown`.

use reqwest::header::{ACCEPT, CONTENT_TYPE, RANGE};
use reqwest::{Method, StatusCode};

use super::announcements::create_user;
use crate::libraries::create;
use crate::support::tasks::wait_for_scan_work;
use crate::support::{TestServer, app_config, json};

#[tokio::test]
async fn admins_see_the_build_version_and_commit() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = TestServer::start_claimed_with(app_config(dir.path())).await;
    create_user(&server, "reader@example.org", &[]).await;

    let response = server
        .admin(Method::GET, "/actuator/info")
        .send()
        .await
        .expect("Info");
    assert_eq!(response.status(), StatusCode::OK);
    let info = json(response).await;
    assert_eq!(info["build"]["name"], "komga");
    let version = info["build"]["version"].as_str().expect("Build version");
    assert!(!version.is_empty());
    // The tests are built from a Git checkout.
    let commit = info["git"]["commit"]["id"].as_str().expect("Commit ID");
    assert!(
        commit.len() == 7 && commit.chars().all(|c| c.is_ascii_hexdigit()),
        "{commit}"
    );
    let time = info["git"]["commit"]["time"].as_str().expect("Commit time");
    assert!(time.ends_with('Z'), "{time}");

    let anonymous = server
        .request(Method::GET, "/actuator/info")
        .send()
        .await
        .expect("Info");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let reader = server
        .request(Method::GET, "/actuator/info")
        .basic_auth("reader@example.org", Some("password"))
        .send()
        .await
        .expect("Info");
    assert_eq!(reader.status(), StatusCode::FORBIDDEN);
    server.stop().await;
}

#[tokio::test]
async fn only_admins_see_the_health_of_each_component() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = TestServer::start_claimed_with(app_config(dir.path())).await;
    create_user(&server, "reader@example.org", &[]).await;

    let response = server
        .admin(Method::GET, "/actuator/health")
        .send()
        .await
        .expect("Health");
    assert_eq!(response.status(), StatusCode::OK);
    let health = json(response).await;
    assert_eq!(health["status"], "UP");
    let components = &health["components"];
    assert_eq!(components["db"]["components"]["main"]["status"], "UP");
    assert_eq!(components["db"]["components"]["tasks"]["status"], "UP");
    assert_eq!(components["diskSpace"]["details"]["exists"], true);
    assert_eq!(components["diskSpace"]["details"]["threshold"], 10_485_760);

    let anonymous = server.request(Method::GET, "/actuator/health");
    let reader = server
        .request(Method::GET, "/actuator/health")
        .basic_auth("reader@example.org", Some("password"));
    for request in [anonymous, reader] {
        let response = request.send().await.expect("Health");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json(response).await, serde_json::json!({ "status": "UP" }));
    }
    let wrong_password = server
        .request(Method::GET, "/actuator/health")
        .basic_auth("reader@example.org", Some("wrong"))
        .send()
        .await
        .expect("Health");
    assert_eq!(wrong_password.status(), StatusCode::UNAUTHORIZED);
    server.stop().await;
}

async fn admin_metric(server: &TestServer, path: &str, status: StatusCode) -> serde_json::Value {
    let response = server
        .admin(Method::GET, path)
        .send()
        .await
        .expect("Metric");
    assert_eq!(response.status(), status, "{path}");
    json(response).await
}

#[tokio::test]
async fn admins_read_entity_counts_and_task_times_by_tag() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    create_user(&server, "reader@example.org", &[]).await;
    let root = directory.path().join("library");
    std::fs::create_dir_all(root.join("series")).expect("Series directory");
    for book in ["1.cbz", "2.cbz"] {
        std::fs::write(root.join("series").join(book), b"book bytes").expect("Book file");
    }
    let library = create(&server, "library", &root).await;
    let library_id = library["id"].as_str().expect("Library ID");
    wait_for_scan_work(directory.path()).await;

    let names = admin_metric(&server, "/actuator/metrics", StatusCode::OK).await;
    assert!(
        names["names"]
            .as_array()
            .expect("Names")
            .contains(&"komga.books".into())
    );
    let books = admin_metric(&server, "/actuator/metrics/komga.books", StatusCode::OK).await;
    assert_eq!(books["baseUnit"], "count");
    assert_eq!(
        books["measurements"],
        serde_json::json!([{"statistic": "VALUE", "value": 2.0}])
    );
    assert_eq!(
        books["availableTags"],
        serde_json::json!([{"tag": "library", "values": [library_id]}])
    );
    let size = admin_metric(
        &server,
        &format!("/actuator/metrics/komga.books.filesize?tag=library:{library_id}"),
        StatusCode::OK,
    )
    .await;
    assert_eq!(size["measurements"][0]["value"], 20.0);
    assert_eq!(size["availableTags"], serde_json::json!([]));
    let scans = admin_metric(
        &server,
        "/actuator/metrics/komga.tasks.execution?tag=type:ScanLibrary",
        StatusCode::OK,
    )
    .await;
    assert_eq!(scans["measurements"][0]["statistic"], "COUNT");
    assert_eq!(scans["measurements"][0]["value"], 1.0);
    let failures = admin_metric(
        &server,
        "/actuator/metrics/komga.tasks.failure",
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        failures["measurements"],
        serde_json::json!([{"statistic": "COUNT", "value": 0.0}])
    );

    for path in [
        "/actuator/metrics/komga.unknown",
        "/actuator/metrics/komga.books?tag=library:unknown",
    ] {
        admin_metric(&server, path, StatusCode::NOT_FOUND).await;
    }
    admin_metric(
        &server,
        "/actuator/metrics/komga.books?tag=library",
        StatusCode::BAD_REQUEST,
    )
    .await;
    let reader = server
        .request(Method::GET, "/actuator/metrics")
        .basic_auth("reader@example.org", Some("password"))
        .send()
        .await
        .expect("Metrics");
    assert_eq!(reader.status(), StatusCode::FORBIDDEN);
    server.stop().await;
}

#[tokio::test]
async fn admins_scrape_http_and_task_meters_labelled_by_route() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    create_user(&server, "reader@example.org", &[]).await;
    let root = directory.path().join("library");
    std::fs::create_dir_all(&root).expect("Library directory");
    let library = create(&server, "library", &root).await;
    let library_id = library["id"].as_str().expect("Library ID");
    wait_for_scan_work(directory.path()).await;
    let path = format!("/api/v1/libraries/{library_id}");
    let response = server
        .admin(Method::GET, &path)
        .send()
        .await
        .expect("Library");
    assert_eq!(response.status(), StatusCode::OK);

    let response = server
        .admin(Method::GET, "/actuator/prometheus")
        .send()
        .await
        .expect("Scrape");
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = &response.headers()["content-type"];
    assert!(
        content_type
            .to_str()
            .expect("ASCII")
            .starts_with("text/plain")
    );
    let scrape = response.text().await.expect("Scrape body");
    let request_line = r#"http_server_requests_seconds_count{method="GET",uri="/api/v1/libraries/{library_id}",status="200"}"#;
    assert!(scrape.contains(request_line), "{scrape}");
    assert!(!scrape.contains(library_id), "{scrape}");
    assert!(
        scrape.contains(r#"komga_tasks_executions_total{type="ScanLibrary"} 1"#),
        "{scrape}"
    );
    assert!(scrape.contains("komga_tasks_queued 0"), "{scrape}");
    assert!(scrape.contains("tokio_workers_count 2"), "{scrape}");
    for metric in [
        "tokio_alive_tasks ",
        "tokio_spawned_tasks_total ",
        "tokio_global_queue_depth ",
        "tokio_local_queue_depth ",
        "tokio_budget_forced_yields_total ",
        "tokio_worker_busy_seconds_total ",
        "tokio_worker_polls_total ",
        "tokio_worker_parks_total ",
        "tokio_worker_steals_total ",
        "tokio_worker_overflows_total ",
        "tokio_blocking_threads ",
        "tokio_idle_blocking_threads ",
        "tokio_blocking_queue_depth ",
        "tokio_io_driver_fd_registered_total ",
        "tokio_io_driver_fd_deregistered_total ",
        "tokio_io_driver_ready_total ",
    ] {
        assert!(scrape.contains(metric), "missing {metric} in:\n{scrape}");
    }

    let reader = server
        .request(Method::GET, "/actuator/prometheus")
        .basic_auth("reader@example.org", Some("password"))
        .send()
        .await
        .expect("Scrape");
    assert_eq!(reader.status(), StatusCode::FORBIDDEN);
    server.stop().await;
}

/// The two endpoints the legacy web UI's server management uses besides `info` and `metrics`.
#[tokio::test]
async fn admins_download_the_log_file_and_stop_the_server() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let config = app_config(dir.path());
    let log_file = config.log_file.value.clone();
    let server = TestServer::start_claimed_with(config).await;
    create_user(&server, "reader@example.org", &[]).await;
    let reader = |method, path| {
        server
            .request(method, path)
            .basic_auth("reader@example.org", Some("password"))
            .send()
    };

    let missing = server.admin(Method::GET, "/actuator/logfile").send().await;
    assert_eq!(missing.expect("Log").status(), StatusCode::NOT_FOUND);
    std::fs::create_dir_all(log_file.parent().expect("Log directory")).expect("Log directory");
    std::fs::write(&log_file, "first line\n").expect("Log file");
    let log = server
        .admin(Method::GET, "/actuator/logfile")
        .send()
        .await
        .expect("Log");
    assert_eq!(log.status(), StatusCode::OK);
    assert_eq!(log.headers()[CONTENT_TYPE], "text/plain;charset=UTF-8");
    assert_eq!(log.text().await.expect("Body"), "first line\n");
    let range = server
        .admin(Method::GET, "/actuator/logfile")
        .header(RANGE, "bytes=0-4")
        .send()
        .await
        .expect("Log");
    assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(range.text().await.expect("Body"), "first");
    let as_json = server
        .admin(Method::GET, "/actuator/logfile")
        .header(ACCEPT, "application/json")
        .send()
        .await;
    assert_eq!(as_json.expect("Log").status(), StatusCode::NOT_ACCEPTABLE);
    let denied = reader(Method::GET, "/actuator/logfile").await;
    assert_eq!(denied.expect("Log").status(), StatusCode::FORBIDDEN);

    let denied = reader(Method::POST, "/actuator/shutdown").await;
    assert_eq!(denied.expect("Shutdown").status(), StatusCode::FORBIDDEN);
    let shutdown = server
        .admin(Method::POST, "/actuator/shutdown")
        .send()
        .await
        .expect("Shutdown");
    assert_eq!(shutdown.status(), StatusCode::OK);
    assert_eq!(json(shutdown).await["message"], "Shutting down, bye...");
    // The server stops by itself; `stop` only waits for it.
    server.stop().await;
}
