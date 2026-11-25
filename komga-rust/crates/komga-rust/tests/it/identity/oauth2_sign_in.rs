use std::path::Path;

use axum::Router;
use axum::http::HeaderMap;
use axum::routing::{get as route_get, post};
use komga_types::UserRole;
use reqwest::StatusCode;
use reqwest::header::{COOKIE, LOCATION, SET_COOKIE};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::oauth2_flow::{assert_failed, get, query, start_login};
use super::persisted_identity;
use crate::support::{ADMIN_EMAIL, TestServer, app_config};

/// A plain OAuth 2.0 provider whose access token is the code, and whose user info has the email
/// `<code>@example.org`, or none for the code `anonymous`.
const CONFIG: &str = r#"
komga.oauth2-account-creation: {account_creation}
spring.security.oauth2.client:
  registration:
    sso:
      provider: sso
      client-id: komga
      client-secret: secret
      client-name: Single Sign-On
      authorization-grant-type: authorization_code
      redirect-uri: "{baseUrl}/{action}/oauth2/code/{registrationId}"
      scope: profile
  provider:
    sso:
      authorization-uri: {provider}/authorize
      token-uri: {provider}/token
      user-info-uri: {provider}/user
      user-name-attribute: id
"#;

async fn token(body: String) -> axum::Json<Value> {
    let code = body
        .split('&')
        .find_map(|pair| pair.strip_prefix("code="))
        .unwrap_or_default();
    axum::Json(json!({"access_token": code, "token_type": "Bearer"}))
}

async fn user(headers: HeaderMap) -> axum::Json<Value> {
    let token = headers["authorization"]
        .to_str()
        .expect("ASCII authorization")
        .trim_start_matches("Bearer ");
    match token {
        "anonymous" => axum::Json(json!({"id": 1})),
        name => axum::Json(json!({"id": 1, "email": format!("{name}@example.org")})),
    }
}

async fn start(dir: &Path, account_creation: bool) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind the provider");
    let provider = format!("http://{}", listener.local_addr().expect("Address"));
    let routes = Router::new()
        .route("/token", post(token))
        .route("/user", route_get(user));
    tokio::spawn(async move { axum::serve(listener, routes).await });
    let config = CONFIG
        .replace("{provider}", &provider)
        .replace("{account_creation}", &account_creation.to_string());
    std::fs::write(dir.join("application.yml"), config).expect("Failed to write config");
    TestServer::start_claimed_with(app_config(dir)).await
}

/// The callback of a login the provider granted with `code`.
async fn sign_in(server: &TestServer, code: &str) -> reqwest::Response {
    let (location, pair, _) = start_login(server, &[]).await;
    let state = &query(&location)["state"];
    let callback = format!("/login/oauth2/code/sso?code={code}&state={state}");
    get(server, &callback, &[(COOKIE.as_str(), &pair)]).await
}

fn session_cookie(response: &reqwest::Response) -> String {
    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("KOMGA-SESSION="))
        .and_then(|value| value.split(';').next())
        .expect("The session cookie must be set")
        .to_owned()
}

#[tokio::test]
async fn known_emails_sign_in_with_a_session() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = start(dir.path(), false).await;
    let reader = super::create_user(&server, "Reader@Example.org", "password").await;

    let response = sign_in(&server, "reader").await;
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(response.headers()[LOCATION], "/?server_redirect=Y");
    let cookie = session_cookie(&response);
    let me = get(&server, "/api/v2/users/me", &[(COOKIE.as_str(), &cookie)]).await;
    assert_eq!(me.status(), StatusCode::OK);
    let me: Value = me.json().await.expect("JSON user");
    assert_eq!(me["id"], reader.as_str());

    assert_failed(&sign_in(&server, "stranger").await, "ERR_1025");
    assert_failed(&sign_in(&server, "anonymous").await, "ERR_1024");
    server.stop().await;

    let identity = persisted_identity(dir.path()).await;
    let activities = super::activities(&identity).await;
    let sources: Vec<_> = activities
        .iter()
        .filter(|activity| activity.source.as_deref() == Some("OAuth2:Single Sign-On"))
        .map(|activity| {
            (
                activity.success,
                activity.user_id.as_deref(),
                activity.error.as_deref(),
            )
        })
        .collect();
    assert_eq!(sources.len(), 3, "{sources:?}");
    assert!(sources.contains(&(true, Some(reader.as_str()), None)));
    assert!(sources.contains(&(false, None, Some("ERR_1025"))));
    assert!(sources.contains(&(false, None, Some("ERR_1024"))));
}

#[tokio::test]
async fn concurrent_first_logins_create_one_user() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = start(dir.path(), true).await;

    let (first, second) = tokio::join!(sign_in(&server, "newcomer"), sign_in(&server, "newcomer"));
    for response in [first, second] {
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(response.headers()[LOCATION], "/?server_redirect=Y");
    }
    server.stop().await;

    let identity = persisted_identity(dir.path()).await;
    let users = identity.list_users().await.expect("Users");
    let emails: Vec<_> = users.iter().map(|user| user.email.as_str()).collect();
    assert_eq!(emails.len(), 2, "{emails:?}");
    assert!(emails.contains(&ADMIN_EMAIL));
    let newcomer = users
        .iter()
        .find(|user| user.email == "newcomer@example.org")
        .expect("The newcomer must have an account");
    assert_eq!(
        newcomer.roles,
        [UserRole::FileDownload, UserRole::PageStreaming].into()
    );
    assert!(newcomer.shared_all_libraries);
}
