use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use komga_api::REMEMBER_ME_COOKIE_NAME;
use reqwest::header::{COOKIE, SET_COOKIE};
use reqwest::{Method, Response, StatusCode};

use super::{get_with, has_expired_remember_cookie, persisted_identity, set_cookie};
use crate::support::{ADMIN_EMAIL, ADMIN_PASSWORD, TestServer, db, json};

const ME: &str = "/api/v2/users/me";

async fn admin_login(server: &TestServer, uri: &str) -> Response {
    let response = server
        .request(Method::GET, uri)
        .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("Login request failed");
    assert_eq!(response.status(), StatusCode::OK);
    response
}

/// The complete `Set-Cookie` value of the remember-me cookie, attributes included.
fn remember_set_cookie(response: &Response) -> Option<String> {
    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{REMEMBER_ME_COOKIE_NAME}=")))
        .map(str::to_owned)
}

/// The expiry, in epoch milliseconds, signed into a Spring remember-me token.
fn token_expiry_millis(token: &str) -> u64 {
    let payload = STANDARD_NO_PAD
        .decode(token)
        .expect("Token must be unpadded Base64");
    let payload = String::from_utf8(payload).expect("Token payload must be UTF-8");
    payload
        .split(':')
        .nth(1)
        .expect("Token must have an expiry")
        .parse()
        .expect("expiry")
}

fn now_millis() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Clock after epoch");
    u64::try_from(now.as_millis()).expect("Epoch milliseconds fit in u64")
}

/// Stops the claimed server, sets, or with `None` removes, its `settings`, and starts it again.
async fn restart_with_settings(
    server: TestServer,
    dir: &Path,
    settings: &[(&str, Option<&str>)],
) -> TestServer {
    server.stop().await;
    let pool = db::main_pool(dir).await;
    db::seed::server_settings(&pool, settings).await;
    pool.close().await;
    TestServer::start(dir).await
}

#[tokio::test]
async fn test_remember_me_activity_and_mixed_credentials() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let login = admin_login(&server, &format!("{ME}?remember-me=true")).await;
    let remember_cookie =
        set_cookie(&login, REMEMBER_ME_COOKIE_NAME).expect("remember-me request must set a cookie");

    // Sessions live in memory, so only the remember-me cookie survives a restart.
    server.stop().await;
    let server = TestServer::start(temp_dir.path()).await;
    let resumed = get_with(&server, ME, &[(COOKIE.as_str(), &remember_cookie)]).await;
    assert_eq!(
        resumed.status(),
        StatusCode::OK,
        "Remember-me must resume auth after restart"
    );
    assert_eq!(json(resumed).await["email"], ADMIN_EMAIL);

    let api_key = server
        .admin(Method::POST, "/api/v2/users/me/api-keys")
        .json(&serde_json::json!({ "comment": "mixed token" }))
        .send()
        .await
        .expect("API-key creation request failed");
    let api_key = json(api_key).await["key"].as_str().expect("key").to_owned();
    let junk_cookie = format!("{REMEMBER_ME_COOKIE_NAME}=invalid_junk_token");
    let headers = [
        ("X-API-Key", api_key.as_str()),
        (COOKIE.as_str(), &junk_cookie),
    ];
    let by_key = get_with(&server, ME, &headers).await;
    assert_eq!(
        by_key.status(),
        StatusCode::OK,
        "X-API-Key must take precedence over the cookie"
    );

    // A password change invalidates the token signature.
    let password = server
        .admin(Method::PATCH, &format!("{ME}/password"))
        .json(&serde_json::json!({ "password": "newsecretpassword" }))
        .send()
        .await
        .expect("Password update request failed");
    assert_eq!(password.status(), StatusCode::NO_CONTENT);
    let revoked = get_with(&server, ME, &[(COOKIE.as_str(), &remember_cookie)]).await;
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
    assert!(
        has_expired_remember_cookie(revoked.headers()),
        "Invalid remember-me tokens must expire the cookie"
    );
    server.stop().await;

    let activities = super::activities(&persisted_identity(temp_dir.path()).await).await;
    for source in ["Password", "RememberMe", "ApiKey"] {
        assert!(
            activities
                .iter()
                .any(|activity| activity.source.as_deref() == Some(source)),
            "{source} login activity should be recorded"
        );
    }
}

#[tokio::test]
async fn basic_auth_issues_remember_cookie_only_when_requested() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    for (uri, requested) in [
        (ME.to_owned(), false),
        (format!("{ME}?remember-me=true"), true),
    ] {
        let response = admin_login(&server, &uri).await;
        assert_eq!(
            remember_set_cookie(&response).is_some(),
            requested,
            "remember-me cookie presence must follow the request parameter"
        );
    }
    server.stop().await;
}

#[tokio::test]
async fn remember_cookie_uses_persisted_settings_and_survives_restart() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let fixture = [
        ("REMEMBER_ME_KEY", None),
        ("REMEMBER_ME_DURATION", Some("2")),
    ];
    let server = restart_with_settings(server, temp_dir.path(), &fixture).await;

    let login = admin_login(&server, &format!("{ME}?remember-me=true")).await;
    let remember_cookie =
        remember_set_cookie(&login).expect("remember-me request must set a persistent cookie");
    assert!(remember_cookie.contains("Max-Age=172800"));
    assert!(remember_cookie.contains("HttpOnly"));
    let remember_token = set_cookie(&login, REMEMBER_ME_COOKIE_NAME)
        .and_then(|pair| pair.split_once('=').map(|(_, token)| token.to_owned()))
        .expect("remember-me cookie must carry a token");
    assert!(
        !remember_token.ends_with('='),
        "Spring removes Base64 padding"
    );

    let decoded = STANDARD_NO_PAD
        .decode(&remember_token)
        .expect("unpadded Base64");
    let decoded = String::from_utf8(decoded).expect("UTF-8 payload");
    let fields: Vec<&str> = decoded.split(':').collect();
    assert_eq!(fields.len(), 4);
    assert_eq!(fields[0], ADMIN_EMAIL.replace('@', "%40"));
    assert_eq!(fields[2], "SHA256");
    assert_eq!(fields[3].len(), 64);
    let validity_millis = 2 * 24 * 60 * 60 * 1000;
    let expiry_millis = token_expiry_millis(&remember_token);
    assert!(expiry_millis > now_millis() + validity_millis - 5000);
    assert!(expiry_millis <= now_millis() + validity_millis);

    // The generated key was persisted: the token still verifies after a restart, padded or not.
    server.stop().await;
    let server = TestServer::start(temp_dir.path()).await;
    let padded = STANDARD.encode(STANDARD_NO_PAD.decode(&remember_token).expect("Base64"));
    let cookie = format!("{REMEMBER_ME_COOKIE_NAME}={padded}");
    let resumed = get_with(&server, ME, &[(COOKIE.as_str(), &cookie)]).await;
    assert_eq!(resumed.status(), StatusCode::OK);
    server.stop().await;
}

#[tokio::test]
async fn negative_remember_duration_uses_session_cookie_and_spring_default_expiry() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let fixture = [("REMEMBER_ME_DURATION", Some("-1"))];
    let server = restart_with_settings(server, temp_dir.path(), &fixture).await;

    let login = admin_login(&server, &format!("{ME}?remember-me=true")).await;
    let cookie = remember_set_cookie(&login).expect("remember-me login must issue a cookie");
    assert!(!cookie.contains("Max-Age="));
    let token = set_cookie(&login, REMEMBER_ME_COOKIE_NAME)
        .and_then(|pair| pair.split_once('=').map(|(_, token)| token.to_owned()))
        .expect("remember-me cookie must carry a token");
    let fourteen_days_millis = 14 * 24 * 60 * 60 * 1000;
    let expiry_millis = token_expiry_millis(&token);
    assert!(expiry_millis > now_millis() + fourteen_days_millis - 5000);
    assert!(expiry_millis <= now_millis() + fourteen_days_millis);
    server.stop().await;
}
