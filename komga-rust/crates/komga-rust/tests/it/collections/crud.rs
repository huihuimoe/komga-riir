//! `POST /api/v1/collections` and `GET|PATCH|DELETE /api/v1/collections/{id}`.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{COLLECTIONS, Fixture, create_user, fixture, rate, send, stored_members};
use crate::support::{json, openapi};

async fn create_collection(fixture: &Fixture, body: &Value) -> Value {
    let response = send(&fixture.server, None, Method::POST, COLLECTIONS, Some(body)).await;
    assert_eq!(response.status(), StatusCode::OK, "{body}");
    openapi::checked(Method::POST, response).await
}

/// The collection `id` as `email` sees it, or `None` when it is not found.
async fn get_collection(fixture: &Fixture, email: Option<&str>, id: &str) -> Option<Value> {
    let path = format!("{COLLECTIONS}/{id}");
    let response = send(&fixture.server, email, Method::GET, &path, None).await;
    match response.status() {
        StatusCode::OK => Some(openapi::checked(Method::GET, response).await),
        StatusCode::NOT_FOUND => None,
        status => panic!("Unexpected collection status {status}"),
    }
}

async fn status(
    fixture: &Fixture,
    email: Option<&str>,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    let response = send(&fixture.server, email, method, path, body).await;
    let status = response.status();
    let body = if status == StatusCode::NO_CONTENT {
        Value::Null
    } else {
        json(response).await
    };
    (status, body)
}

fn id(collection: &Value) -> &str {
    collection["id"].as_str().expect("Collection ID")
}

#[tokio::test]
async fn admins_create_collections_in_the_given_order() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let series = [&f.b, &f.a2, &f.a1];
    let created = create_collection(&f, &json!({"name": "Set", "seriesIds": series})).await;
    assert_eq!(created["name"], "Set");
    assert_eq!(created["ordered"], false, "ordered defaults to false");
    assert_eq!(created["seriesIds"], json!(series));
    assert_eq!(created["filtered"], false);
    let stored = stored_members(&f.pool, id(&created)).await;
    let expected = vec![(0, f.b.clone()), (1, f.a2.clone()), (2, f.a1.clone())];
    assert_eq!(stored, (3, expected));

    let post = |body: Value| {
        let f = &f;
        async move { status(f, None, Method::POST, COLLECTIONS, Some(&body)).await }
    };
    let (code, body) = post(json!({"name": "SET", "seriesIds": [&f.a1]})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Collection name already exists");
    let (code, _) = post(json!({"name": "Other", "seriesIds": [&f.a1, "missing"]})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "an unknown series");
    let (code, _) = post(json!({"name": "Other"})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "seriesIds are required");
    let (code, body) = post(json!({"name": " ", "seriesIds": []})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let mut violations = body["violations"].as_array().expect("Violations").clone();
    violations.sort_by_key(|violation| violation["fieldName"].to_string());
    assert_eq!(
        violations,
        [
            json!({"fieldName": "name", "message": "must not be blank"}),
            json!({"fieldName": "seriesIds", "message": "must not be empty"}),
        ]
    );
    let (code, body) = post(json!({"name": "Other", "seriesIds": [&f.a1, &f.a1]})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["violations"],
        json!([{"fieldName": "seriesIds", "message": "must only contain unique elements"}])
    );

    create_user(&f.server, "reader@example.com", json!({})).await;
    let reader = Some("reader@example.com");
    let valid = json!({"name": "Other", "seriesIds": [&f.a1]});
    let (code, _) = status(&f, reader, Method::POST, COLLECTIONS, Some(&valid)).await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let invalid = json!({"name": ""});
    let (code, _) = status(&f, reader, Method::POST, COLLECTIONS, Some(&invalid)).await;
    assert_eq!(
        code,
        StatusCode::BAD_REQUEST,
        "the body is read before the role check"
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM COLLECTION")
        .fetch_one(&f.pool)
        .await
        .expect("Collections");
    assert_eq!(count, 1, "no rejected request creates a collection");
    f.server.stop().await;
}

#[tokio::test]
async fn users_see_only_the_series_they_may_access() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let body = json!({"name": "Set", "seriesIds": [&f.b, &f.a2, &f.a1]});
    let created = create_collection(&f, &body).await;
    let id = id(&created);
    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.com", library_a).await;
    let no_library = json!({"sharedLibraries": {"all": false, "libraryIds": []}});
    create_user(&f.server, "none@example.com", no_library).await;
    let allow_12 = json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}});
    create_user(&f.server, "twelve@example.com", allow_12).await;
    rate(&f.server, &f.a1, 18).await;
    rate(&f.server, &f.b, 6).await;

    let seen = get_collection(&f, Some("a@example.com"), id)
        .await
        .expect("Visible");
    assert_eq!(seen["seriesIds"], json!([&f.a2, &f.a1]));
    assert_eq!(seen["filtered"], true);
    let seen = get_collection(&f, Some("twelve@example.com"), id)
        .await
        .expect("Visible");
    assert_eq!(seen["seriesIds"], json!([&f.b]));
    assert_eq!(seen["filtered"], true);
    assert_eq!(get_collection(&f, Some("none@example.com"), id).await, None);
    let seen = get_collection(&f, None, id).await.expect("Visible");
    assert_eq!(
        seen["seriesIds"], created["seriesIds"],
        "the admin sees every series"
    );
    assert_eq!(get_collection(&f, None, "missing").await, None);
    f.server.stop().await;
}

#[tokio::test]
async fn updates_replace_every_member_or_none() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let created = create_collection(&f, &json!({"name": "X", "seriesIds": [&f.a1, &f.a2]})).await;
    create_collection(&f, &json!({"name": "Y", "seriesIds": [&f.b]})).await;
    let path = format!("{COLLECTIONS}/{}", id(&created));
    let patch = |email: Option<&'static str>, body: Value| {
        let path = path.clone();
        let f = &f;
        async move { status(f, email, Method::PATCH, &path, Some(&body)).await }
    };

    let (code, body) = patch(None, json!({"name": "y"})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Collection name already exists");
    let (code, _) = patch(None, json!({"seriesIds": [&f.a2, &f.b, "missing"]})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let (code, body) = patch(None, json!({"name": "", "seriesIds": []})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["violations"].as_array().map(Vec::len),
        Some(2),
        "{body}"
    );
    let unchanged = (2, vec![(0, f.a1.clone()), (1, f.a2.clone())]);
    assert_eq!(stored_members(&f.pool, id(&created)).await, unchanged);

    let body = json!({"name": "x", "ordered": true, "seriesIds": [&f.b, &f.a1]});
    let (code, _) = patch(None, body).await;
    assert_eq!(
        code,
        StatusCode::NO_CONTENT,
        "recasing its own name is allowed"
    );
    let updated = get_collection(&f, None, id(&created))
        .await
        .expect("Updated");
    assert_eq!(
        (&updated["name"], &updated["ordered"], &updated["seriesIds"]),
        (&json!("x"), &json!(true), &json!([&f.b, &f.a1]))
    );
    assert!(updated["lastModifiedDate"].as_str() >= created["lastModifiedDate"].as_str());
    let replaced = (2, vec![(0, f.b.clone()), (1, f.a1.clone())]);
    assert_eq!(stored_members(&f.pool, id(&created)).await, replaced);

    // An admin who sees only `b` changes the order without dropping `a1`.
    rate(&f.server, &f.a1, 18).await;
    rate(&f.server, &f.b, 6).await;
    let restricted =
        json!({"roles": ["ADMIN"], "ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}});
    create_user(&f.server, "restricted@example.com", restricted).await;
    let (code, _) = patch(Some("restricted@example.com"), json!({"ordered": false})).await;
    assert_eq!(code, StatusCode::NO_CONTENT);
    assert_eq!(stored_members(&f.pool, id(&created)).await, replaced);

    create_user(&f.server, "reader@example.com", json!({})).await;
    let (code, _) = patch(Some("reader@example.com"), json!({})).await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let missing = format!("{COLLECTIONS}/missing");
    let (code, _) = status(&f, None, Method::PATCH, &missing, Some(&json!({}))).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    f.server.stop().await;
}

#[tokio::test]
async fn deleting_a_collection_removes_its_members() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let created = create_collection(&f, &json!({"name": "X", "seriesIds": [&f.a1, &f.b]})).await;
    let path = format!("{COLLECTIONS}/{}", id(&created));
    create_user(&f.server, "reader@example.com", json!({})).await;

    let reader = Some("reader@example.com");
    let (code, _) = status(&f, reader, Method::DELETE, &path, None).await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = status(&f, None, Method::DELETE, &path, None).await;
    assert_eq!(code, StatusCode::NO_CONTENT);
    assert_eq!(get_collection(&f, None, id(&created)).await, None);
    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM COLLECTION_SERIES")
        .fetch_one(&f.pool)
        .await
        .expect("Members");
    assert_eq!(members, 0);
    let (code, _) = status(&f, None, Method::DELETE, &path, None).await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    f.server.stop().await;
}
