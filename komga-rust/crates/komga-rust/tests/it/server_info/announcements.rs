//! `AnnouncementController`: the website feed, cached, with each admin's read state.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::StatusCode as SiteStatus;
use axum::response::IntoResponse;
use axum::routing::get;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{fake_site, serve, server};
use crate::support::{TestServer, json};

const PATH: &str = "/api/v1/announcements";

fn feed() -> Value {
    json!({
        "version": "https://jsonfeed.org/version/1",
        "title": "Komga",
        "home_page_url": "https://komga.org/blog",
        "favicon": "https://komga.org/favicon.ico",
        "items": [
            {
                "id": "drop-2",
                "url": "https://komga.org/blog/drop-2",
                "title": "Drop 2",
                "content_html": "<p>Drop 2</p>",
                "date_modified": "2023-12-15T02:00:00+02:00",
                "author": {"name": "gotson"},
                "tags": ["upgrade", "komga"],
                "_komga": {"read": true}
            },
            {"id": "drop-1"}
        ]
    })
}

pub(super) async fn create_user(server: &TestServer, email: &str, roles: &[&str]) {
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&json!({"email": email, "password": "password", "roles": roles}))
        .send()
        .await
        .expect("User creation request failed");
    assert_eq!(response.status(), StatusCode::CREATED);
}

async fn get_as(server: &TestServer, email: &str) -> reqwest::Response {
    server
        .request(Method::GET, PATH)
        .basic_auth(email, Some("password"))
        .send()
        .await
        .expect("Announcements request failed")
}

fn read_flags(feed: &Value) -> Vec<(&str, bool)> {
    feed["items"]
        .as_array()
        .expect("Feed items")
        .iter()
        .map(|item| {
            let id = item["id"].as_str().expect("Item ID");
            (id, item["_komga"]["read"].as_bool().expect("Read flag"))
        })
        .collect()
}

#[tokio::test]
async fn the_feed_is_fetched_once_and_read_per_admin() {
    let (site, hits) = fake_site(|_| axum::Json(feed()).into_response()).await;
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = server(&site, dir.path()).await;
    create_user(&server, "other@example.org", &["ADMIN"]).await;
    create_user(&server, "reader@example.org", &[]).await;

    let response = server.admin(Method::GET, PATH).send().await.expect("Get");
    assert_eq!(response.status(), StatusCode::OK);
    let fetched = json(response).await;
    assert_eq!(read_flags(&fetched), [("drop-2", false), ("drop-1", false)]);
    assert_eq!(fetched["favicon"], Value::Null);
    assert_eq!(fetched["description"], Value::Null);
    let item = &fetched["items"][0];
    assert_eq!(item["date_modified"], "2023-12-15T00:00:00Z");
    assert_eq!(item["author"], json!({"name": "gotson", "url": null}));
    assert_eq!(item["tags"], json!(["upgrade", "komga"]));
    assert_eq!(fetched["items"][1]["tags"], json!([]));

    for _ in 0..2 {
        let marked = server
            .admin(Method::PUT, PATH)
            .json(&json!(["drop-2", "drop-2"]))
            .send()
            .await
            .expect("Mark read");
        assert_eq!(marked.status(), StatusCode::NO_CONTENT);
    }
    let mine = json(server.admin(Method::GET, PATH).send().await.expect("Get")).await;
    assert_eq!(read_flags(&mine), [("drop-2", true), ("drop-1", false)]);
    let other = json(get_as(&server, "other@example.org").await).await;
    assert_eq!(read_flags(&other), [("drop-2", false), ("drop-1", false)]);
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    assert_eq!(
        get_as(&server, "reader@example.org").await.status(),
        StatusCode::FORBIDDEN
    );
    let reader_put = |body: &'static str| {
        server
            .request(Method::PUT, PATH)
            .basic_auth("reader@example.org", Some("password"))
            .header("content-type", "application/json")
            .body(body)
            .send()
    };
    let invalid = reader_put("{").await.expect("Put");
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let forbidden = reader_put("[\"drop-1\"]").await.expect("Put");
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    server.stop().await;
}

#[tokio::test]
async fn failed_fetches_are_not_cached_but_empty_ones_are() {
    let (site, hits) = fake_site(|n| match n {
        0 => SiteStatus::INTERNAL_SERVER_ERROR.into_response(),
        1 => axum::Json(json!({"title": "No version"})).into_response(),
        2 => SiteStatus::OK.into_response(),
        _ => axum::Json(feed()).into_response(),
    })
    .await;
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = server(&site, dir.path()).await;
    let get = || async { server.admin(Method::GET, PATH).send().await.expect("Get") };

    assert_eq!(get().await.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(get().await.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(get().await.status(), StatusCode::NOT_FOUND);
    assert_eq!(get().await.status(), StatusCode::NOT_FOUND);
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    server.stop().await;
}

#[tokio::test]
async fn a_slow_website_fails_within_the_timeout() {
    let site = serve(Router::new().route(
        "/feed",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            axum::Json(feed())
        }),
    ))
    .await;
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = server(&site, dir.path()).await;

    let started = Instant::now();
    let response = server.admin(Method::GET, PATH).send().await.expect("Get");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    server.stop().await;
}
