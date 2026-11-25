//! `ReleaseController`: GitHub's latest releases, cached, for admins.

use std::sync::atomic::Ordering;

use axum::http::StatusCode as SiteStatus;
use axum::response::IntoResponse;
use reqwest::{Method, StatusCode};
use serde_json::json;

use super::{fake_site, server};
use crate::support::json;

const PATH: &str = "/api/v1/releases";

#[tokio::test]
async fn releases_are_mapped_cached_and_admin_only() {
    let (site, hits) = fake_site(|_| {
        axum::Json(json!([
            {
                "html_url": "https://github.com/huihuimoe/komga-riir/releases/tag/0.0.39",
                "tag_name": "0.0.39",
                "published_at": "2025-11-02T10:00:00+01:00",
                "body": "Fixes",
                "prerelease": false,
                "draft": false
            },
            {
                "html_url": "https://github.com/huihuimoe/komga-riir/releases/tag/0.0.39-rc.1",
                "tag_name": "0.0.39-rc.1",
                "published_at": "2025-10-01T10:00:00Z",
                "body": "",
                "prerelease": true
            }
        ]))
        .into_response()
    })
    .await;
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = server(&site, dir.path()).await;

    for _ in 0..2 {
        let response = server.admin(Method::GET, PATH).send().await.expect("Get");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            json(response).await,
            json!([
                {
                    "version": "0.0.39",
                    "releaseDate": "2025-11-02T09:00:00Z",
                    "url": "https://github.com/huihuimoe/komga-riir/releases/tag/0.0.39",
                    "latest": true,
                    "preRelease": false,
                    "description": "Fixes"
                },
                {
                    "version": "0.0.39-rc.1",
                    "releaseDate": "2025-10-01T10:00:00Z",
                    "url": "https://github.com/huihuimoe/komga-riir/releases/tag/0.0.39-rc.1",
                    "latest": false,
                    "preRelease": true,
                    "description": ""
                }
            ])
        );
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    super::announcements::create_user(&server, "reader@example.org", &[]).await;
    let forbidden = server
        .request(Method::GET, PATH)
        .basic_auth("reader@example.org", Some("password"))
        .send()
        .await
        .expect("Get");
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    server.stop().await;
}

#[tokio::test]
async fn a_failed_fetch_is_an_error_and_an_empty_one_cached_no_releases() {
    let (site, hits) = fake_site(|n| match n {
        0 => SiteStatus::FORBIDDEN.into_response(),
        _ => SiteStatus::OK.into_response(),
    })
    .await;
    let dir = komga_test_support::tempdir().expect("Failed to create config directory");
    let server = server(&site, dir.path()).await;
    let get = || async { server.admin(Method::GET, PATH).send().await.expect("Get") };

    assert_eq!(get().await.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let empty = get().await;
    assert_eq!(empty.status(), StatusCode::OK);
    assert_eq!(json(empty).await, json!([]));
    assert_eq!(json(get().await).await, json!([]));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    server.stop().await;
}
