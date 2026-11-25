use komga_api::{REMEMBER_ME_COOKIE_NAME, SESSION_COOKIE_NAME};
use komga_service::IdentityService;
use reqwest::header;
use reqwest::{Method, Response, StatusCode};

use super::{basic_auth, create_user, has_expired_remember_cookie, persisted_identity, set_cookie};
use crate::support::{ADMIN_EMAIL, ADMIN_PASSWORD, TestServer, json};

mod activity;
mod remember_me_rejection;

struct CredentialContext {
    server: TestServer,
    identity: IdentityService,
    admin_id: String,
    reader_id: String,
    admin_basic: String,
    reader_basic: String,
    invalid_admin_basic: String,
    reader_api_key: String,
    reader_api_key_id: String,
    _temp_dir: komga_test_support::TestDir,
}

#[tokio::test]
async fn mixed_credentials_follow_session_basic_api_key_remember_me_order() {
    let context = credential_context().await;
    let (admin_session, admin_remember) = login_with_remember(
        &context,
        &context.admin_basic,
        "203.0.113.10",
        "basic-client/1.0",
    )
    .await;

    assert_credential_precedence(&context, &admin_session, &admin_remember).await;
    assert_invalid_credentials_do_not_downgrade(&context, &admin_remember).await;
    assert_authenticated_as(
        &context,
        vec![(header::COOKIE.as_str(), admin_remember)],
        ADMIN_EMAIL,
    )
    .await;
    activity::assert_authentication_activity(&context).await;
}

async fn assert_credential_precedence(
    context: &CredentialContext,
    admin_session: &str,
    admin_remember: &str,
) {
    let cases = [
        (
            vec![
                (
                    header::COOKIE.as_str(),
                    format!("{admin_session}; {admin_remember}"),
                ),
                (header::AUTHORIZATION.as_str(), context.reader_basic.clone()),
                ("X-API-Key", context.reader_api_key.clone()),
            ],
            ADMIN_EMAIL,
        ),
        (
            vec![
                (header::AUTHORIZATION.as_str(), context.admin_basic.clone()),
                ("X-API-Key", context.reader_api_key.clone()),
                (header::COOKIE.as_str(), admin_remember.to_owned()),
            ],
            ADMIN_EMAIL,
        ),
        (
            vec![
                (header::AUTHORIZATION.as_str(), context.reader_basic.clone()),
                (header::COOKIE.as_str(), admin_remember.to_owned()),
            ],
            "reader@example.com",
        ),
        (
            vec![
                ("X-API-Key", context.reader_api_key.clone()),
                (header::COOKIE.as_str(), admin_remember.to_owned()),
                (header::USER_AGENT.as_str(), "api-key-client/1.0".to_owned()),
                ("X-Real-IP", "203.0.113.13".to_owned()),
            ],
            "reader@example.com",
        ),
        (
            vec![
                (header::COOKIE.as_str(), admin_remember.to_owned()),
                (
                    header::USER_AGENT.as_str(),
                    "remember-client/1.0".to_owned(),
                ),
                ("X-Real-IP", "203.0.113.12".to_owned()),
            ],
            ADMIN_EMAIL,
        ),
    ];
    for (headers, expected_email) in cases {
        assert_authenticated_as(context, headers, expected_email).await;
    }
}

async fn assert_invalid_credentials_do_not_downgrade(
    context: &CredentialContext,
    admin_remember: &str,
) {
    let invalid_basic = send_request(
        context,
        Method::GET,
        "/api/v2/users/me",
        vec![
            (
                header::AUTHORIZATION.as_str(),
                context.invalid_admin_basic.clone(),
            ),
            ("X-API-Key", context.reader_api_key.clone()),
            (header::COOKIE.as_str(), admin_remember.to_owned()),
            (
                header::USER_AGENT.as_str(),
                "bad-password-client/1.0".to_owned(),
            ),
            ("X-Real-IP", "203.0.113.14".to_owned()),
        ],
    )
    .await;
    assert_eq!(invalid_basic.status(), StatusCode::UNAUTHORIZED);
    assert!(
        !has_expired_remember_cookie(invalid_basic.headers()),
        "An earlier credential failure must not erase an unvalidated remember-me cookie"
    );

    let unknown_user = send_request(
        context,
        Method::GET,
        "/api/v2/users/me",
        vec![
            (
                header::AUTHORIZATION.as_str(),
                basic_auth("unknown@example.com", "wrong-password"),
            ),
            (header::COOKIE.as_str(), admin_remember.to_owned()),
            (
                header::USER_AGENT.as_str(),
                "unknown-password-client/1.0".to_owned(),
            ),
            ("X-Real-IP", "203.0.113.16".to_owned()),
        ],
    )
    .await;
    assert_eq!(unknown_user.status(), StatusCode::UNAUTHORIZED);
    assert!(
        !has_expired_remember_cookie(unknown_user.headers()),
        "An earlier credential failure must not erase an unvalidated remember-me cookie"
    );

    let invalid_api_key = send_request(
        context,
        Method::GET,
        "/api/v2/users/me",
        vec![
            ("X-API-Key", "invalid-api-key".to_owned()),
            (header::COOKIE.as_str(), admin_remember.to_owned()),
            (header::USER_AGENT.as_str(), "bad-key-client/1.0".to_owned()),
            ("X-Real-IP", "203.0.113.15".to_owned()),
        ],
    )
    .await;
    assert_eq!(invalid_api_key.status(), StatusCode::UNAUTHORIZED);
    assert!(
        !has_expired_remember_cookie(invalid_api_key.headers()),
        "An API-key failure must not erase an unvalidated remember-me cookie"
    );
}

async fn credential_context() -> CredentialContext {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create identity test directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let reader_id = create_user(&server, "reader@example.com", "readerpassword").await;
    let api_key = server
        .request(Method::POST, "/api/v2/users/me/api-keys")
        .basic_auth("reader@example.com", Some("readerpassword"))
        .json(&serde_json::json!({ "comment": "reader key" }))
        .send()
        .await
        .expect("API-key creation request failed");
    assert_eq!(api_key.status(), StatusCode::OK);
    let api_key = json(api_key).await;
    let identity = persisted_identity(temp_dir.path()).await;
    let admin_id = identity
        .find_by_email(ADMIN_EMAIL)
        .await
        .expect("Failed to read admin")
        .expect("Admin must exist")
        .id
        .to_string();

    CredentialContext {
        _temp_dir: temp_dir,
        server,
        identity,
        admin_id,
        reader_id,
        admin_basic: basic_auth(ADMIN_EMAIL, ADMIN_PASSWORD),
        reader_basic: basic_auth("reader@example.com", "readerpassword"),
        invalid_admin_basic: basic_auth(ADMIN_EMAIL, "wrong-password"),
        reader_api_key: api_key["key"].as_str().expect("key").to_owned(),
        reader_api_key_id: api_key["id"].as_str().expect("id").to_owned(),
    }
}

async fn login_with_remember(
    context: &CredentialContext,
    basic_auth: &str,
    ip: &str,
    user_agent: &str,
) -> (String, String) {
    let response = send_request(
        context,
        Method::GET,
        "/api/v2/users/me?remember-me=true",
        vec![
            (header::AUTHORIZATION.as_str(), basic_auth.to_owned()),
            (header::USER_AGENT.as_str(), user_agent.to_owned()),
            ("X-Real-IP", ip.to_owned()),
        ],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    (
        cookie_pair(&response, SESSION_COOKIE_NAME),
        cookie_pair(&response, REMEMBER_ME_COOKIE_NAME),
    )
}

async fn send_request(
    context: &CredentialContext,
    method: Method,
    uri: &str,
    headers: Vec<(&str, String)>,
) -> Response {
    let mut request = context.server.request(method, uri);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    request.send().await.expect("Authentication request failed")
}

async fn assert_authenticated_as(
    context: &CredentialContext,
    headers: Vec<(&str, String)>,
    expected_email: &str,
) {
    let response = send_request(context, Method::GET, "/api/v2/users/me", headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["email"], expected_email);
}

fn cookie_pair(response: &Response, name: &str) -> String {
    set_cookie(response, name).expect("Authentication response must include the requested cookie")
}

fn tamper_cookie_value(cookie: &str) -> String {
    let (name, value) = cookie
        .split_once('=')
        .expect("Remember-me cookie must contain a key/value pair");
    let mut bytes = value.as_bytes().to_vec();
    let index = bytes
        .len()
        .checked_sub(8)
        .expect("Remember-me token must be long enough to contain a signature");
    bytes[index] = if bytes[index] == b'A' { b'B' } else { b'A' };
    let value = String::from_utf8(bytes).expect("Base64 token remains valid UTF-8");
    format!("{name}={value}")
}
