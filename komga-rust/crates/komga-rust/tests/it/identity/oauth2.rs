use axum::routing::get;
use axum::{Json, Router};
use reqwest::{Method, StatusCode};
use serde_json::json;
use tokio::net::TcpListener;

use crate::support::{TestServer, app_config, json};

/// Serves the OpenID Connect discovery document of an issuer at its own address.
async fn issuer() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind the issuer");
    let issuer = format!("http://{}", listener.local_addr().expect("Issuer address"));
    let metadata = json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "jwks_uri": format!("{issuer}/jwks"),
        "userinfo_endpoint": format!("{issuer}/userinfo"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
    });
    let app = Router::new().route(
        "/.well-known/openid-configuration",
        get(move || async move { Json(metadata) }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await });
    issuer
}

#[tokio::test]
async fn sign_in_page_lists_the_configured_providers() {
    let issuer = issuer().await;
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let yaml = format!(
        r#"
spring.security.oauth2.client:
  registration:
    github:
      client-id: gh
      client-secret: secret
    sso:
      provider: sso
      client-id: komga
      scope: openid,email
  provider:
    sso:
      issuer-uri: {issuer}
"#
    );
    std::fs::write(dir.path().join("application.yml"), yaml).expect("Failed to write config");
    let server = TestServer::start_with(app_config(dir.path())).await;

    let response = server
        .request(Method::GET, "/api/v1/oauth2/providers")
        .send()
        .await
        .expect("Providers request failed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json(response).await,
        json!([
            {"name": "GitHub", "registrationId": "github"},
            {"name": issuer, "registrationId": "sso"},
        ])
    );
    server.stop().await;
}

#[tokio::test]
async fn sign_in_page_lists_no_providers_by_default() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = TestServer::start(dir.path()).await;

    let response = server
        .request(Method::GET, "/api/v1/oauth2/providers")
        .send()
        .await
        .expect("Providers request failed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await, json!([]));
    server.stop().await;
}

#[tokio::test]
async fn unreachable_issuer_stops_startup() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("Failed to reserve a port");
    let issuer = format!(
        "http://{}",
        listener.local_addr().expect("Reserved address")
    );
    drop(listener);
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let yaml = format!(
        "spring.security.oauth2.client:\n  registration.sso:\n    provider: sso\n    client-id: komga\n  provider.sso.issuer-uri: {issuer}\n"
    );
    std::fs::write(dir.path().join("application.yml"), yaml).expect("Failed to write config");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral HTTP port");

    let error = komga_rust::app::serve_with_shutdown(
        app_config(dir.path()),
        listener,
        std::future::pending(),
    )
    .await
    .expect_err("An unreachable issuer must stop startup");
    assert_eq!(
        error.to_string(),
        "Failed to resolve the OAuth2 client registrations"
    );
}
