//! The Kobo store proxy (`KoboProxy`): what is forwarded, what comes back, and what never leaves.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method as StoreMethod, StatusCode as StoreStatus, Uri};
use axum::response::{IntoResponse, Response};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::api_key;
use crate::books::epub::sample;
use crate::books::library_with_books_configured;
use crate::support::{TestServer, app_config, json};

/// A request the fake store received.
#[derive(Debug, Clone)]
struct Received {
    method: StoreMethod,
    uri: String,
    headers: HeaderMap,
    body: Bytes,
}

type Log = Arc<Mutex<Vec<Received>>>;

/// A local stand-in for `storeapi.kobo.com`; returns its URL and what it receives.
async fn fake_store() -> (String, Log) {
    let log = Log::default();
    let recorded = log.clone();
    let app = Router::new().fallback(
        move |method: StoreMethod, uri: Uri, headers: HeaderMap, body: Bytes| {
            let log = recorded.clone();
            async move {
                log.lock()
                    .expect("Fake store or Kobo request")
                    .push(Received {
                        method,
                        uri: uri.to_string(),
                        headers: headers.clone(),
                        body: body.clone(),
                    });
                store_answer(uri.path(), &headers, body).await
            }
        },
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Fake store or Kobo request");
    let url = format!(
        "http://{}",
        listener.local_addr().expect("Fake store or Kobo request")
    );
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("Fake store or Kobo request")
    });
    (url, log)
}

async fn store_answer(path: &str, headers: &HeaderMap, body: Bytes) -> Response {
    let authorization = headers
        .get("authorization")
        .map(|value| value.to_str().expect("Fake store or Kobo request"));
    let token = headers
        .get("x-kobo-synctoken")
        .map(|value| value.to_str().expect("Fake store or Kobo request"));
    match (path, authorization) {
        (_, Some("Bearer expired")) => StoreStatus::UNAUTHORIZED.into_response(),
        (_, Some("Bearer broken")) => StoreStatus::INTERNAL_SERVER_ERROR.into_response(),
        ("/v1/initialization", _) => {
            axum::Json(json!({"Resources": {"library_sync": "store"}})).into_response()
        }
        ("/v1/library/sync", _) => {
            let (next, more) = match token {
                Some("store-1") => ("store-2", None),
                _ => ("store-1", Some(("x-kobo-sync", "continue"))),
            };
            let headers = [("x-kobo-synctoken", next)].into_iter().chain(more);
            let headers: Vec<_> = headers.collect();
            (
                axum::response::AppendHeaders(headers),
                axum::Json(json!([{"NewEntitlement": {"Store": next}}])),
            )
                .into_response()
        }
        ("/missing", _) => StoreStatus::NOT_FOUND.into_response(),
        ("/redirect", _) => (
            StoreStatus::FOUND,
            [("location", "http://127.0.0.1:9/elsewhere")],
        )
            .into_response(),
        ("/slow", _) => {
            tokio::time::sleep(Duration::from_secs(2)).await;
            axum::Json(json!({})).into_response()
        }
        ("/text", _) => "not json".into_response(),
        _ => (
            [("x-kobo-recent", "yes"), ("set-cookie", "store=1")],
            axum::Json(json!({"Echo": String::from_utf8_lossy(&body)})),
        )
            .into_response(),
    }
}

/// A claimed server with the store proxy on, a Kobo key and one novel.
async fn proxied(directory: &Path, store: &str, limit: u32) -> (TestServer, String) {
    std::fs::write(
        directory.join("application.yml"),
        format!("komga:\n  kobo:\n    sync-item-limit: {limit}\n"),
    )
    .expect("Fake store or Kobo request");
    let mut config = app_config(directory);
    config.kobo_store_url = store.to_owned();
    config.kobo_store_timeout = Duration::from_millis(500);
    let novel = sample("The Incomplete Theft - Ralph Burke.epub");
    let books = [
        ("Series/One.epub", novel.clone()),
        ("Series/Two.epub", novel),
    ];
    let (server, _, _) = library_with_books_configured(config, &books).await;
    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({"koboProxy": true}))
        .send()
        .await
        .expect("Fake store or Kobo request");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let key = api_key(&server, None).await;
    (server, key)
}

fn received(log: &Log) -> Vec<Received> {
    std::mem::take(&mut *log.lock().expect("Fake store or Kobo request"))
}

#[tokio::test]
async fn unknown_requests_reach_the_store_with_device_headers_only() {
    let directory = komga_test_support::tempdir().expect("Fake store or Kobo request");
    let (store, log) = fake_store().await;
    let (server, key) = proxied(directory.path(), &store, 100).await;

    let response = server
        .request(
            Method::PUT,
            &format!("/kobo/{key}/v1/user/profile?a=1&b=%20"),
        )
        .header("Authorization", "Bearer device")
        .header("X-Kobo-DeviceId", "device")
        .header("X-Kobo-SyncToken", "KOMGA.secret")
        .header("Cookie", "KOMGA-SESSION=secret")
        .header("X-Forwarded-Host", "komga.local")
        .header("Content-Type", "application/json")
        .body(r#"{"Name":"me"}"#)
        .send()
        .await
        .expect("Fake store or Kobo request");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-kobo-recent"], "yes");
    assert!(!response.headers().contains_key("set-cookie"));
    assert_eq!(json(response).await, json!({"Echo": r#"{"Name":"me"}"#}));

    let [request] = &received(&log)[..] else {
        panic!("one store request")
    };
    assert_eq!(request.method, StoreMethod::PUT);
    assert_eq!(request.uri, "/v1/user/profile?a=1&b=%20");
    assert_eq!(request.body, r#"{"Name":"me"}"#);
    let mut names: Vec<_> = request.headers.keys().map(|name| name.as_str()).collect();
    names.retain(|name| !["host", "content-length", "accept"].contains(name));
    names.sort_unstable();
    assert_eq!(names, ["authorization", "content-type", "x-kobo-deviceid"]);
}

#[tokio::test]
async fn store_failures_are_answered_without_following_or_waiting() {
    let directory = komga_test_support::tempdir().expect("Fake store or Kobo request");
    let (store, log) = fake_store().await;
    let (server, key) = proxied(directory.path(), &store, 100).await;
    let status = |path: &'static str| {
        let request = server.request(Method::GET, &format!("/kobo/{key}{path}"));
        async move {
            request
                .send()
                .await
                .expect("Fake store or Kobo request")
                .status()
        }
    };
    assert_eq!(status("/missing").await, StatusCode::NOT_FOUND);
    assert_eq!(status("/redirect").await, StatusCode::BAD_GATEWAY);
    assert_eq!(status("/text").await, StatusCode::BAD_GATEWAY);
    let started = std::time::Instant::now();
    assert_eq!(status("/slow").await, StatusCode::BAD_GATEWAY);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the store timeout applies"
    );
    let paths: Vec<_> = received(&log)
        .into_iter()
        .map(|request| request.uri)
        .collect();
    assert_eq!(
        paths,
        ["/missing", "/redirect", "/text", "/slow"],
        "redirects are not followed"
    );
}

#[tokio::test]
async fn the_store_answers_for_books_komga_does_not_have() {
    let directory = komga_test_support::tempdir().expect("Fake store or Kobo request");
    let (store, log) = fake_store().await;
    let (server, key) = proxied(directory.path(), &store, 100).await;
    let base = format!("/kobo/{key}/v1/library/store-book");
    for (method, path, body) in [
        (Method::GET, format!("{base}/metadata"), ""),
        (Method::GET, format!("{base}/state"), ""),
        (
            Method::PUT,
            format!("{base}/state"),
            r#"{"ReadingStates":[]}"#,
        ),
    ] {
        let response = server
            .request(method, &path)
            .body(body)
            .send()
            .await
            .expect("Fake store or Kobo request");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(json(response).await, json!({"Echo": body}));
    }
    let requests = received(&log);
    let paths: Vec<_> = requests
        .iter()
        .map(|request| request.uri.as_str())
        .collect();
    let store_base = "/v1/library/store-book";
    assert_eq!(
        paths,
        [
            format!("{store_base}/metadata"),
            format!("{store_base}/state"),
            format!("{store_base}/state")
        ]
    );
}

#[tokio::test]
async fn initialization_and_device_auth_prefer_the_store() {
    let directory = komga_test_support::tempdir().expect("Fake store or Kobo request");
    let (store, log) = fake_store().await;
    let (server, key) = proxied(directory.path(), &store, 100).await;
    let initialization = |authorization: &'static str| {
        server
            .request(Method::GET, &format!("/kobo/{key}/v1/initialization"))
            .header("Authorization", authorization)
            .send()
    };

    let resources = json(
        initialization("Bearer device")
            .await
            .expect("Fake store or Kobo request"),
    )
    .await["Resources"]
        .clone();
    assert_eq!(resources["library_sync"], "store");
    let local = format!("http://127.0.0.1:{}", server.port());
    assert_eq!(resources["image_host"], local.as_str());
    let response = initialization("Bearer expired")
        .await
        .expect("Fake store or Kobo request");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "the device must sign in again"
    );
    let resources = json(
        initialization("Bearer broken")
            .await
            .expect("Fake store or Kobo request"),
    )
    .await["Resources"]
        .clone();
    assert_eq!(
        resources["library_sync"], "https://storeapi.kobo.com/v1/library/sync",
        "native resources when the store fails"
    );

    let auth = |authorization: &'static str| {
        server
            .request(Method::POST, &format!("/kobo/{key}/v1/auth/device"))
            .header("Authorization", authorization)
            .body(r#"{"UserKey":"user"}"#)
            .send()
    };
    let stored = json(
        auth("Bearer device")
            .await
            .expect("Fake store or Kobo request"),
    )
    .await;
    assert_eq!(stored, json!({"Echo": r#"{"UserKey":"user"}"#}));
    let dummy = json(
        auth("Bearer broken")
            .await
            .expect("Fake store or Kobo request"),
    )
    .await;
    assert_eq!(
        dummy["UserKey"], "user",
        "a dummy authentication when the store fails"
    );
    assert_eq!(received(&log).len(), 5);
}

async fn sync(
    server: &TestServer,
    key: &str,
    token: Option<&str>,
    auth: &str,
) -> (Value, String, bool) {
    let mut request = server
        .request(Method::GET, &format!("/kobo/{key}/v1/library/sync"))
        .header("Authorization", auth);
    if let Some(token) = token {
        request = request.header("X-Kobo-SyncToken", token);
    }
    let response = request.send().await.expect("Fake store or Kobo request");
    assert_eq!(response.status(), StatusCode::OK);
    let token = response.headers()["x-kobo-synctoken"]
        .to_str()
        .expect("Fake store or Kobo request")
        .to_owned();
    let more = response.headers().contains_key("x-kobo-sync");
    (json(response).await, token, more)
}

fn sources(items: &Value) -> Vec<&str> {
    items
        .as_array()
        .expect("Fake store or Kobo request")
        .iter()
        .map(|item| match item["NewEntitlement"]["Store"].as_str() {
            Some(_) => "store",
            None => "komga",
        })
        .collect()
}

#[tokio::test]
async fn the_store_sync_follows_the_last_komga_page() {
    let directory = komga_test_support::tempdir().expect("Fake store or Kobo request");
    let (store, log) = fake_store().await;
    let (server, key) = proxied(directory.path(), &store, 1).await;

    let (items, token, more) = sync(&server, &key, None, "Bearer device").await;
    assert_eq!((sources(&items), more), (vec!["komga"], true));
    assert!(
        received(&log).is_empty(),
        "the store waits for the last Komga page"
    );

    let (items, token, more) = sync(&server, &key, Some(&token), "Bearer device").await;
    assert_eq!((sources(&items), more), (vec!["komga", "store"], true));
    let requests = received(&log);
    assert!(!requests[0].headers.contains_key("x-kobo-synctoken"));

    let (items, token, more) = sync(&server, &key, Some(&token), "Bearer device").await;
    assert_eq!((sources(&items), more), (vec!["store"], false));
    assert_eq!(received(&log)[0].headers["x-kobo-synctoken"], "store-1");

    let (items, _, more) = sync(&server, &key, Some(&token), "Bearer broken").await;
    assert_eq!(
        (sources(&items), more),
        (vec![], false),
        "a failing store adds nothing"
    );
    assert_eq!(received(&log)[0].headers["x-kobo-synctoken"], "store-2");
}

#[tokio::test]
async fn without_the_proxy_nothing_reaches_the_store() {
    let directory = komga_test_support::tempdir().expect("Fake store or Kobo request");
    let (store, log) = fake_store().await;
    let (server, key) = proxied(directory.path(), &store, 100).await;
    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({"koboProxy": false}))
        .send()
        .await
        .expect("Fake store or Kobo request");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    for (method, path) in [
        (Method::GET, "/v1/initialization"),
        (Method::GET, "/v1/library/sync"),
        (Method::DELETE, "/v1/anything"),
    ] {
        let response = server
            .request(method, &format!("/kobo/{key}{path}"))
            .send()
            .await
            .expect("Fake store or Kobo request");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
    let response = server
        .request(
            Method::GET,
            &format!("/kobo/{key}/v1/library/store-book/metadata"),
        )
        .send()
        .await
        .expect("Fake store or Kobo request");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json(response).await,
        json!([]),
        "an admin sees no such book locally"
    );
    assert!(received(&log).is_empty());
}
