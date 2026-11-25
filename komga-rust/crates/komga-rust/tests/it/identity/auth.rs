use komga_api::{REMEMBER_ME_COOKIE_NAME, SESSION_COOKIE_NAME};
use reqwest::header::{COOKIE, SET_COOKIE};
use reqwest::{Method, StatusCode};

use super::{get_with, set_cookie};
use crate::support::{ADMIN_EMAIL, ADMIN_PASSWORD, TestServer, app_config};

const ME: &str = "/api/v2/users/me";

#[tokio::test]
async fn session_cookie_max_age_follows_the_session_timeout() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let mut config = app_config(temp_dir.path());
    config.session_timeout.value = std::time::Duration::from_secs(3600);
    let server = TestServer::start_claimed_with(config).await;

    let login = server
        .request(Method::GET, ME)
        .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("Request failed");
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = login
        .headers()
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("Basic authentication must set the session cookie");
    assert!(cookie.contains("Max-Age=3600"), "{cookie}");
}

#[tokio::test]
async fn basic_auth_finds_an_email_with_non_ascii_capitals() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    super::create_user(&server, "Élodie@example.com", "secret").await;

    // SQLite `lower` folds only ASCII, so both sides must go through it.
    let login = server
        .request(Method::GET, ME)
        .basic_auth("Élodie@EXAMPLE.com", Some("secret"))
        .send()
        .await
        .expect("Request failed");
    assert_eq!(login.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_basic_auth_and_session_lifecycle() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start_claimed(temp_dir.path()).await;

    let anonymous = get_with(&server, ME, &[]).await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    // Wrong Basic credentials are rejected and do not create a session.
    let wrong = server
        .request(Method::GET, ME)
        .basic_auth(ADMIN_EMAIL, Some("wrongpass"))
        .send()
        .await
        .expect("Request failed");
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert!(wrong.headers().get(SET_COOKIE).is_none());

    // Basic authentication with X-Auth-Token creates and returns a header session.
    let basic = server
        .admin(Method::GET, ME)
        .header("X-Auth-Token", "new-session")
        .send()
        .await
        .expect("Request failed");
    assert_eq!(basic.status(), StatusCode::OK);
    assert!(basic.headers().get(SET_COOKIE).is_none());
    let auth_token = basic
        .headers()
        .get("X-Auth-Token")
        .expect("Basic authentication must return the session header")
        .to_str()
        .expect("Session header must be ASCII")
        .to_owned();
    let by_header = get_with(&server, ME, &[("X-Auth-Token", &auth_token)]).await;
    assert_eq!(by_header.status(), StatusCode::OK);

    // The baseline endpoint converts a header session to the KOMGA-SESSION cookie.
    let set_cookie_response = get_with(
        &server,
        "/api/v1/login/set-cookie",
        &[("X-Auth-Token", &auth_token)],
    )
    .await;
    assert_eq!(set_cookie_response.status(), StatusCode::NO_CONTENT);
    let attributes = set_cookie_response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("Set-cookie endpoint must set a cookie");
    assert!(attributes.contains("Max-Age=604800"), "{attributes}");
    let session_cookie = set_cookie(&set_cookie_response, SESSION_COOKIE_NAME)
        .expect("Set-cookie endpoint must return the session cookie");
    assert_eq!(
        session_cookie,
        format!("{SESSION_COOKIE_NAME}={auth_token}")
    );
    // A sent X-Auth-Token is authoritative and never falls back to the cookie.
    let headers = [
        ("X-Auth-Token", "invalid-session"),
        (COOKIE.as_str(), &session_cookie),
    ];
    let invalid_header = get_with(&server, ME, &headers).await;
    assert_eq!(invalid_header.status(), StatusCode::UNAUTHORIZED);

    // The deprecated hand-written login endpoint is not exposed.
    let login = server
        .admin(Method::POST, "/api/v1/login")
        .send()
        .await
        .expect("Request failed");
    assert_eq!(login.status(), StatusCode::NOT_FOUND);

    let logout = server
        .request(Method::DELETE, "/api/logout")
        .header("X-Auth-Token", &auth_token)
        .send()
        .await
        .expect("Request failed");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    let expired_remember_cookie = logout
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with(&format!("{REMEMBER_ME_COOKIE_NAME}=")))
        .expect("logout must cancel the remember-me cookie");
    assert!(expired_remember_cookie.contains("Max-Age=0"));
    assert!(!expired_remember_cookie.contains("HttpOnly"));

    // Both header and cookie references to the revoked session are rejected.
    let by_header = get_with(&server, ME, &[("X-Auth-Token", &auth_token)]).await;
    assert_eq!(by_header.status(), StatusCode::UNAUTHORIZED);
    let by_cookie = get_with(&server, ME, &[(COOKIE.as_str(), &session_cookie)]).await;
    assert_eq!(by_cookie.status(), StatusCode::UNAUTHORIZED);
    server.stop().await;
}

#[tokio::test]
async fn test_basic_request_creates_reusable_cookie_session() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start_claimed(temp_dir.path()).await;

    let basic = server
        .request(Method::GET, ME)
        .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("Request failed");
    assert_eq!(basic.status(), StatusCode::OK);
    assert!(basic.headers().get("X-Auth-Token").is_none());
    let session_cookie = set_cookie(&basic, SESSION_COOKIE_NAME)
        .expect("Basic authentication must create a KOMGA session cookie");

    let cookie = [(COOKIE.as_str(), session_cookie.as_str())];
    assert_eq!(
        get_with(&server, ME, &cookie).await.status(),
        StatusCode::OK
    );
    // Browsers are redirected to the login page (followed here), API clients get 204.
    let browser_logout = server
        .request(Method::GET, "/api/logout")
        .header("Accept", "text/html")
        .send()
        .await
        .expect("Request failed");
    assert_eq!(browser_logout.url().path(), "/login");
    assert_eq!(browser_logout.url().query(), Some("logout"));
    let logout = get_with(&server, "/api/logout", &cookie).await;
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        get_with(&server, ME, &cookie).await.status(),
        StatusCode::UNAUTHORIZED
    );
    server.stop().await;
}
