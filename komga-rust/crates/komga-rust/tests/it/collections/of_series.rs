//! `GET /api/v1/series/{id}/collections`.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{COLLECTIONS, Fixture, create_user, fixture, rate, send};
use crate::support::openapi;

async fn create(f: &Fixture, name: &str, series: &[&str]) -> String {
    let body = json!({"name": name, "seriesIds": series});
    let response = send(&f.server, None, Method::POST, COLLECTIONS, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::OK, "{name}");
    let collection: Value = response.json().await.expect("Collection");
    collection["id"].as_str().expect("Collection ID").to_owned()
}

/// The collections of `series` as `email` sees them, or the failure status.
async fn of_series(f: &Fixture, email: Option<&str>, series: &str) -> Result<Value, StatusCode> {
    let path = format!("/api/v1/series/{series}/collections");
    let response = send(&f.server, email, Method::GET, &path, None).await;
    match response.status() {
        StatusCode::OK => Ok(openapi::checked(Method::GET, response).await),
        status => Err(status),
    }
}

fn ids(collections: &Value) -> Vec<&str> {
    let mut ids: Vec<&str> = collections
        .as_array()
        .expect("Collection array")
        .iter()
        .map(|collection| collection["id"].as_str().expect("Collection ID"))
        .collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn collections_of_a_series_show_only_visible_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let both = create(&f, "Both", &[&f.b, &f.a1]).await;
    let only_a = create(&f, "Only A", &[&f.a1, &f.a2]).await;
    create(&f, "Only B", &[&f.b]).await;

    let collections = of_series(&f, None, &f.a1).await.expect("Collections");
    let mut expected = [both.as_str(), only_a.as_str()];
    expected.sort_unstable();
    assert_eq!(ids(&collections), expected);
    let of_a2 = of_series(&f, None, &f.a2).await.expect("Collections");
    assert_eq!(ids(&of_a2), [only_a.as_str()]);
    assert_eq!(of_series(&f, None, "missing").await, Ok(json!([])));

    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.com", library_a).await;
    let seen = of_series(&f, Some("a@example.com"), &f.a1)
        .await
        .expect("Collections");
    let both_seen = seen
        .as_array()
        .expect("Collection array")
        .iter()
        .find(|collection| collection["id"] == both.as_str())
        .expect("Both collection");
    assert_eq!(both_seen["seriesIds"], json!([&f.a1]));
    assert_eq!(both_seen["filtered"], true);
    let hidden = of_series(&f, Some("a@example.com"), &f.b).await;
    assert_eq!(hidden, Err(StatusCode::FORBIDDEN));
    let missing = of_series(&f, Some("a@example.com"), "missing").await;
    assert_eq!(missing, Err(StatusCode::NOT_FOUND));

    let allow_12 = json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}});
    create_user(&f.server, "twelve@example.com", allow_12).await;
    rate(&f.server, &f.b, 6).await;
    assert_eq!(
        of_series(&f, Some("twelve@example.com"), &f.a1).await,
        Err(StatusCode::FORBIDDEN)
    );
    let seen = of_series(&f, Some("twelve@example.com"), &f.b)
        .await
        .expect("Collections");
    assert_eq!(seen.as_array().map(Vec::len), Some(2));
    let both_seen = seen
        .as_array()
        .expect("Collection array")
        .iter()
        .find(|collection| collection["id"] == both.as_str())
        .expect("Both collection");
    assert_eq!(both_seen["seriesIds"], json!([&f.b]));
    let unauthenticated = f
        .server
        .request(Method::GET, &format!("/api/v1/series/{}/collections", f.a1))
        .send()
        .await
        .expect("Request");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    f.server.stop().await;
}
