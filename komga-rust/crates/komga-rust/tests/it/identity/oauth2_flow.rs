use std::collections::HashMap;
use std::path::Path;

use axum::Router;
use axum::http::StatusCode as AxumStatus;
use axum::routing::post;
use reqwest::header::{COOKIE, LOCATION, SET_COOKIE};
use reqwest::redirect::Policy;
use reqwest::{Client, Response, StatusCode, Url};
use serde_json::json;
use tokio::net::TcpListener;

use crate::support::{TestServer, app_config};

const FLOW_COOKIE: &str = "KOMGA-OAUTH2-FLOW";

const CONFIG: &str = r#"
spring.security.oauth2.client:
  registration:
    sso:
      provider: sso
      client-id: komga
      client-secret: secret
      authorization-grant-type: authorization_code
      redirect-uri: "{baseUrl}/{action}/oauth2/code/{registrationId}"
      scope: openid,email
  provider:
    sso:
      authorization-uri: https://sso.test/authorize?prompt=login
      token-uri: {token_uri}
      jwk-set-uri: https://sso.test/jwks
"#;

async fn start(dir: &Path) -> TestServer {
    start_with_token_uri(dir, "https://sso.test/token").await
}

async fn start_with_token_uri(dir: &Path, token_uri: &str) -> TestServer {
    let config = CONFIG.replace("{token_uri}", token_uri);
    std::fs::write(dir.join("application.yml"), config).expect("Failed to write config");
    TestServer::start_with(app_config(dir)).await
}

/// A GET as the browser sends it, without following the redirect.
pub(super) async fn get(server: &TestServer, path: &str, headers: &[(&str, &str)]) -> Response {
    let client = Client::builder()
        .redirect(Policy::none())
        .build()
        .expect("HTTP client");
    let mut request = client.get(format!("http://127.0.0.1:{}{path}", server.port()));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    request.send().await.expect("Request failed")
}

fn flow_set_cookie(response: &Response) -> String {
    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{FLOW_COOKIE}=")))
        .expect("The flow cookie must be set")
        .to_owned()
}

/// Starts a login and returns the provider URL and the `Cookie` header that goes back with it.
pub(super) async fn start_login(
    server: &TestServer,
    headers: &[(&str, &str)],
) -> (Url, String, String) {
    let response = get(server, "/oauth2/authorization/sso", headers).await;
    assert_eq!(response.status(), StatusCode::FOUND);
    let location = Url::parse(
        response.headers()[LOCATION]
            .to_str()
            .expect("ASCII location"),
    )
    .expect("The provider URL must be absolute");
    let set_cookie = flow_set_cookie(&response);
    let pair = set_cookie
        .split(';')
        .next()
        .expect("Cookie pair")
        .to_owned();
    (location, pair, set_cookie)
}

pub(super) fn query(url: &Url) -> HashMap<String, String> {
    url.query_pairs().into_owned().collect()
}

/// The callback must send the browser back to the login page and clear the flow cookie.
pub(super) fn assert_failed(response: &Response, error: &str) {
    assert_eq!(response.status(), StatusCode::FOUND, "{error}");
    assert_eq!(
        response.headers()[LOCATION],
        format!("/login?server_redirect=Y&error={error}").as_str()
    );
    let removal = flow_set_cookie(response);
    assert!(removal.contains("Max-Age=0"), "{removal}");
}

#[tokio::test]
async fn sign_in_redirects_to_the_provider_with_an_encrypted_flow_cookie() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = start(dir.path()).await;

    let (location, pair, set_cookie) = start_login(&server, &[]).await;
    assert_eq!(location.host_str(), Some("sso.test"));
    assert_eq!(location.path(), "/authorize");
    let params = query(&location);
    assert_eq!(params["prompt"], "login");
    assert_eq!(params["response_type"], "code");
    assert_eq!(params["client_id"], "komga");
    assert_eq!(params["scope"], "openid email");
    assert_eq!(
        params["redirect_uri"],
        format!("http://127.0.0.1:{}/login/oauth2/code/sso", server.port())
    );
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(params.contains_key("code_challenge") && params.contains_key("nonce"));
    for attribute in ["Path=/", "HttpOnly", "SameSite=Lax", "Max-Age=300"] {
        assert!(set_cookie.contains(attribute), "{set_cookie}");
    }
    assert!(!set_cookie.contains("Secure"), "{set_cookie}");
    assert!(
        !pair.contains(&params["state"]),
        "The state must not be readable"
    );

    let (location, _, set_cookie) = start_login(
        &server,
        &[
            ("X-Forwarded-Proto", "https"),
            ("X-Forwarded-Host", "komga.example"),
        ],
    )
    .await;
    assert_eq!(
        query(&location)["redirect_uri"],
        "https://komga.example/login/oauth2/code/sso"
    );
    assert!(set_cookie.contains("Secure"), "{set_cookie}");

    for path in ["/oauth2/authorization/other", "/login/oauth2/code/other"] {
        assert_eq!(
            get(&server, path, &[]).await.status(),
            StatusCode::NOT_FOUND
        );
    }
    server.stop().await;
}

#[tokio::test]
async fn callbacks_without_the_matching_login_go_back_to_the_login_page() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = start(dir.path()).await;
    let (location, pair, _) = start_login(&server, &[]).await;
    let state = query(&location)["state"].clone();
    let mut tampered = pair.clone().into_bytes();
    let last = tampered.last_mut().expect("Cookie value");
    *last = if *last == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered).expect("ASCII cookie");

    let cases = [
        (
            format!("code=c&state={state}"),
            None,
            "authorization_request_not_found",
        ),
        (
            "code=c&state=other".to_owned(),
            Some(&pair),
            "authorization_request_not_found",
        ),
        (
            format!("code=c&state={state}"),
            Some(&tampered),
            "authorization_request_not_found",
        ),
        ("code=c".to_owned(), Some(&pair), "invalid_request"),
        (format!("state={state}"), Some(&pair), "invalid_request"),
        (
            format!("error=access_denied&state={state}"),
            Some(&pair),
            "access_denied",
        ),
    ];
    for (params, cookie, error) in cases {
        let headers: Vec<_> = cookie
            .iter()
            .map(|cookie| (COOKIE.as_str(), cookie.as_str()))
            .collect();
        let response = get(
            &server,
            &format!("/login/oauth2/code/sso?{params}"),
            &headers,
        )
        .await;
        assert_failed(&response, error);
    }
    server.stop().await;
}

#[tokio::test]
async fn a_login_in_progress_survives_a_restart() {
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = start(dir.path()).await;
    let (location, pair, _) = start_login(&server, &[]).await;
    server.stop().await;

    let server = start(dir.path()).await;
    let state = &query(&location)["state"];
    let response = get(
        &server,
        &format!("/login/oauth2/code/sso?error=access_denied&state={state}"),
        &[(COOKIE.as_str(), &pair)],
    )
    .await;
    assert_failed(&response, "access_denied");
    server.stop().await;
}

/// A token endpoint that refuses the code `refused` and answers others without an ID token.
async fn token(body: String) -> (AxumStatus, axum::Json<serde_json::Value>) {
    if body.contains("code=refused") {
        let error = json!({"error": "invalid_grant"});
        (AxumStatus::BAD_REQUEST, axum::Json(error))
    } else {
        let tokens = json!({"access_token": "at", "token_type": "Bearer"});
        (AxumStatus::OK, axum::Json(tokens))
    }
}

#[tokio::test]
async fn failed_exchanges_go_back_to_the_login_page_with_the_error() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind the token endpoint");
    let token_uri = format!("http://{}/token", listener.local_addr().expect("Address"));
    let provider = Router::new().route("/token", post(token));
    tokio::spawn(async move { axum::serve(listener, provider).await });
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = start_with_token_uri(dir.path(), &token_uri).await;

    for (code, error) in [
        ("refused", "invalid_grant"),
        ("granted", "invalid_id_token"),
    ] {
        let (location, pair, _) = start_login(&server, &[]).await;
        let state = &query(&location)["state"];
        let response = get(
            &server,
            &format!("/login/oauth2/code/sso?code={code}&state={state}"),
            &[(COOKIE.as_str(), &pair)],
        )
        .await;
        assert_failed(&response, error);
    }
    server.stop().await;
}
