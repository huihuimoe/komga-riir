//! Kotlin `IndexController`, the UI resources of `WebMvcConfiguration`, and
//! `ResourceNotFoundController`.

use reqwest::header::{CACHE_CONTROL, CONTENT_TYPE};
use reqwest::{Method, Response, StatusCode};
use serde_json::json;

use crate::support::TestServer;

fn header(response: &Response, name: &reqwest::header::HeaderName) -> String {
    let value = response.headers().get(name).expect("Header");
    value.to_str().expect("ASCII header").to_owned()
}

async fn page(server: &TestServer, path: &str) -> String {
    let response = server.request(Method::GET, path).send().await.expect(path);
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    assert_eq!(header(&response, &CACHE_CONTROL), "no-store", "{path}");
    assert!(header(&response, &CONTENT_TYPE).starts_with("text/html"));
    response.text().await.expect("Page")
}

#[tokio::test]
#[cfg_attr(not(web_ui_built), ignore = "needs komga-webui/dist and next-ui/dist")]
async fn navigations_render_the_chosen_ui_and_files_are_served() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let server = TestServer::start_claimed(config_dir.path()).await;
    let classic = page(&server, "/").await;
    assert!(
        classic.contains("window.resourceBaseUrl = '/'"),
        "{classic}"
    );
    assert!(!classic.contains("th:inline"));
    for path in [
        "/series/0F99E2PSBK0FZ/books",
        "/classic/settings",
        "/classic/",
        "/index.html",
    ] {
        assert_eq!(page(&server, path).await, classic, "{path}");
    }
    let next = page(&server, "/next/libraries").await;
    assert_ne!(next, classic);
    assert_eq!(page(&server, "/index-next.html").await, next);
    assert_eq!(page(&server, "/next/").await, next);

    let script = classic
        .split("src=\"")
        .skip(1)
        .find_map(|rest| rest.split('"').next().filter(|src| src.starts_with("/js/")))
        .expect("The relative script path resolved against /");
    let file = server
        .request(Method::GET, script)
        .send()
        .await
        .expect(script);
    assert_eq!(file.status(), StatusCode::OK);
    assert_eq!(header(&file, &CACHE_CONTROL), "max-age=31536000, public");
    assert!(header(&file, &CONTENT_TYPE).contains("javascript"));
    // An unread body would hold up the server's graceful shutdown.
    file.bytes().await.expect("Script");
    let map = format!("{script}.map");
    let map = server.request(Method::GET, &map).send().await.expect("Map");
    assert_eq!(
        map.status(),
        StatusCode::NOT_FOUND,
        "Source maps are not embedded"
    );
    let icon = server
        .request(Method::HEAD, "/favicon.ico")
        .send()
        .await
        .expect("Icon");
    assert_eq!(icon.status(), StatusCode::OK);
    assert_eq!(header(&icon, &CACHE_CONTROL), "no-store");

    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({ "nextUiAsDefault": true }))
        .send()
        .await
        .expect("Settings update");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(page(&server, "/").await, next);
    assert_eq!(page(&server, "/libraries").await, next);
}

#[tokio::test]
async fn unmatched_requests_other_than_navigations_are_not_found() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let server = TestServer::start_claimed(config_dir.path()).await;
    for path in ["/api/v1/unknown", "/opds", "/sse/v1/unknown"] {
        let anonymous = server.request(Method::GET, path).send().await.expect(path);
        assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED, "{path}");
        let response = server.admin(Method::GET, path).send().await.expect(path);
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        let body: serde_json::Value = response.json().await.expect("Error body");
        assert_eq!(body["path"], path);
    }
    for (method, path) in [
        (Method::GET, "/mstile-150x150.png"),
        (Method::GET, "/js/missing.js"),
        (Method::GET, "/actuator/unknown"),
        (Method::GET, "/koreader/unknown"),
        (Method::GET, "/v3/api-docs"),
        (Method::GET, "/swagger-ui/index.html"),
    ] {
        let response = server.request(method, path).send().await.expect(path);
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert!(
            !header(&response, &CONTENT_TYPE).starts_with("text/html"),
            "{path}"
        );
    }
}
