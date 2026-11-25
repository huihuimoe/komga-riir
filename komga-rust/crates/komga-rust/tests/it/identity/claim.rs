use reqwest::{Method, StatusCode};
use serde_json::json;

use super::{assert_default_error, assert_tsid_id};
use crate::support::{TestServer, db, json};

fn claim(server: &TestServer, email: &str, password: &str) -> reqwest::RequestBuilder {
    server
        .request(Method::POST, "/api/v1/claim")
        .header("X-Komga-Email", email)
        .header("X-Komga-Password", password)
}

async fn is_claimed(server: &TestServer) -> serde_json::Value {
    let response = server
        .request(Method::GET, "/api/v1/claim")
        .send()
        .await
        .expect("Claim status request failed");
    assert_eq!(response.status(), StatusCode::OK);
    json(response).await["isClaimed"].clone()
}

async fn me_status(server: &TestServer, email: &str, password: &str) -> StatusCode {
    server
        .request(Method::GET, "/api/v2/users/me")
        .basic_auth(email, Some(password))
        .send()
        .await
        .expect("Current user request failed")
        .status()
}

#[tokio::test]
async fn test_claim_http_endpoint_lifecycle() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start(temp_dir.path()).await;
    assert_eq!(is_claimed(&server).await, false);

    let response = claim(&server, "admin@example.com", "adminpassword123")
        .send()
        .await
        .expect("Claim request failed");
    assert_eq!(response.status(), StatusCode::OK);
    let user = json(response).await;
    assert_tsid_id(&user["id"]);
    assert_eq!(user["email"], "admin@example.com");
    assert_eq!(user["sharedAllLibraries"], true);
    let roles = user["roles"].as_array().expect("roles must be an array");
    assert!(roles.iter().any(|role| role == "ADMIN"));
    let ok = me_status(&server, "admin@example.com", "adminpassword123");
    assert_eq!(ok.await, StatusCode::OK);
    let wrong = me_status(&server, "admin@example.com", "wrong-password");
    assert_eq!(wrong.await, StatusCode::UNAUTHORIZED);
    assert_eq!(is_claimed(&server).await, true);

    let response = claim(&server, "second@example.com", "anotherpassword")
        .send()
        .await
        .expect("Repeated claim request failed");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_default_error(
        &json(response).await,
        400,
        "This server has already been claimed",
        "/api/v1/claim",
    );
    server.stop().await;
}

#[tokio::test]
async fn invalid_claim_credentials_are_violations() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start(temp_dir.path()).await;

    // `a.b@c` has a dot, but none after the `@`.
    let response = claim(&server, "a.b@c", " ").send().await;
    let response = response.expect("Claim request failed");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let violations = json!([
        {"fieldName": "claimServer.email", "message": "must be a well-formed email address"},
        {"fieldName": "claimServer.password", "message": "must not be blank"},
    ]);
    assert_eq!(json(response).await["violations"], violations);
    assert_eq!(is_claimed(&server).await, false);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_claim_creates_exactly_one_initial_admin() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start(temp_dir.path()).await;

    let (first, second) = tokio::join!(
        claim(&server, "first@example.com", "first-password").send(),
        claim(&server, "second@example.com", "second-password").send(),
    );
    let statuses = [
        first.expect("First claim request failed").status(),
        second.expect("Second claim request failed").status(),
    ];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1,
        "exactly one concurrent claim may succeed, got {statuses:?}"
    );
    assert!(statuses.contains(&StatusCode::BAD_REQUEST), "{statuses:?}");
    server.stop().await;

    let pool = db::main_pool(temp_dir.path()).await;
    let emails = db::user_emails(&pool).await;
    assert_eq!(emails.len(), 1);
    assert!(
        matches!(
            emails[0].as_str(),
            "first@example.com" | "second@example.com"
        ),
        "The persisted admin must be one of the competing claimants, got {emails:?}"
    );
}
