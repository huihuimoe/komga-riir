//! Spring `ForwardedHeaderFilter` and the Tomcat request URL in front of every route.

use komga_rust::config::{ConfigSource, Setting};
use reqwest::header::SET_COOKIE;
use reqwest::{Method, Response, StatusCode};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::support::{ADMIN_EMAIL, ADMIN_PASSWORD, TestServer, app_config, json};

#[tokio::test]
async fn unreadable_forwarded_headers_are_rejected_before_authentication() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let server = TestServer::start(config_dir.path()).await;
    for (path, name, value) in [
        ("/opds/v2/auth", "X-Forwarded-Port", "x"),
        ("/api/v1/books", "X-Forwarded-Proto", ","),
        ("/", "Forwarded", "host=a:b:c"),
    ] {
        let response = server
            .request(Method::GET, path)
            .header(name, value)
            .send()
            .await
            .expect(path);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        let body = json(response).await;
        assert_eq!(
            (&body["error"], &body["message"], &body["path"]),
            (
                &json!("Bad Request"),
                &json!("No message available"),
                &json!(path)
            ),
        );
    }
    server.stop().await;
}

/// HTTP/1.0 lets clients leave `Host` out; links then name the address the server listens on.
#[tokio::test]
async fn requests_without_host_link_to_the_local_address() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let server = TestServer::start(config_dir.path()).await;
    let mut stream = TcpStream::connect(("127.0.0.1", server.port()))
        .await
        .expect("Connection");
    stream
        .write_all(b"GET /opds/v2/auth HTTP/1.0\r\n\r\n")
        .await
        .expect("Request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("Response");
    assert!(response.starts_with("HTTP/1.0 200"), "{response}");
    let expected = format!("\"http://127.0.0.1:{}/opds/v2/", server.port());
    assert!(response.contains(&expected), "{response}");
    server.stop().await;
}

fn cookies(response: &Response) -> Vec<&str> {
    let values = response.headers().get_all(SET_COOKIE).iter();
    values
        .map(|value| value.to_str().expect("ASCII cookie"))
        .collect()
}

#[tokio::test]
async fn a_context_path_prefixes_routes_links_cookies_and_the_ui() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let mut config = app_config(config_dir.path());
    config.context_path = Some(Setting {
        value: "/komga".to_owned(),
        source: ConfigSource::Default,
    });
    let server = TestServer::start_with(config).await;
    let claim = server
        .request(Method::POST, "/komga/api/v1/claim")
        .header("X-Komga-Email", ADMIN_EMAIL)
        .header("X-Komga-Password", ADMIN_PASSWORD)
        .send()
        .await
        .expect("Claim");
    assert_eq!(claim.status(), StatusCode::OK);
    let outside = server.request(Method::GET, "/api/v1/claim").send().await;
    assert_eq!(outside.expect("Outside").status(), StatusCode::NOT_FOUND);

    let me = server
        .request(Method::GET, "/komga/api/v2/users/me?remember-me=true")
        .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("Login");
    assert_eq!(me.status(), StatusCode::OK);
    let cookies = cookies(&me);
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("KOMGA-SESSION=") && c.contains("Path=/komga/")),
        "{cookies:?}"
    );
    assert!(
        cookies
            .iter()
            .any(|c| c.starts_with("komga-remember-me=") && c.contains("Path=/komga;")),
        "{cookies:?}"
    );

    let settings = server
        .request(Method::GET, "/komga/api/v1/settings")
        .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("Settings");
    let settings = json(settings).await;
    assert_eq!(
        settings["serverContextPath"],
        json!({"configurationSource": "/komga", "databaseSource": null, "effectiveValue": "/komga"})
    );

    let auth = server
        .request(Method::GET, "/komga/opds/v2/auth")
        .send()
        .await;
    let auth = json(auth.expect("Authentication document"))
        .await
        .to_string();
    let link = format!("http://127.0.0.1:{}/komga/opds/v2/", server.port());
    assert!(auth.contains(&link), "{auth}");

    if cfg!(web_ui_built) {
        let root = server.request(Method::GET, "/komga").send().await;
        let root = root.expect("Context root");
        assert_eq!(
            (root.status(), root.url().path()),
            (StatusCode::OK, "/komga/")
        );
        root.bytes().await.expect("Page body");
        let page = server.request(Method::GET, "/komga/series/1").send().await;
        let page = page.expect("Page").text().await.expect("Page body");
        assert!(
            page.contains("window.resourceBaseUrl = '/komga/'"),
            "{page}"
        );
        assert!(page.contains("src=\"/komga/js/"), "{page}");
    }
    server.stop().await;
}

#[tokio::test]
async fn forwarded_https_marks_the_remember_me_cookie_secure() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let server = TestServer::start_claimed(config_dir.path()).await;
    for (proto, secure) in [("https", true), ("http", false)] {
        let me = server
            .request(Method::GET, "/api/v2/users/me?remember-me=true")
            .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
            .header("X-Forwarded-Proto", proto)
            .send()
            .await
            .expect("Login");
        let cookies = cookies(&me);
        let remember_me = cookies
            .iter()
            .find(|c| c.starts_with("komga-remember-me="))
            .expect("Remember-me cookie");
        assert_eq!(remember_me.contains("; Secure"), secure, "{remember_me}");
        let session = cookies
            .iter()
            .find(|c| c.starts_with("KOMGA-SESSION="))
            .expect("Session cookie");
        assert_eq!(session.contains("; Secure"), secure, "{session}");
    }
    server.stop().await;
}
