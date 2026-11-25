//! `POST /api/v1/readlists` and `GET|PATCH|DELETE /api/v1/readlists/{id}`.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{Fixture, READ_LISTS, create_user, fixture, rate, send, stored_members};
use crate::support::{json, openapi};

async fn create_list(fixture: &Fixture, body: &Value) -> Value {
    let response = send(&fixture.server, None, Method::POST, READ_LISTS, Some(body)).await;
    assert_eq!(response.status(), StatusCode::OK, "{body}");
    openapi::checked(Method::POST, response).await
}

/// The read list `id` as `email` sees it, or `None` when it is not found.
async fn get_list(fixture: &Fixture, email: Option<&str>, id: &str) -> Option<Value> {
    let path = format!("{READ_LISTS}/{id}");
    let response = send(&fixture.server, email, Method::GET, &path, None).await;
    match response.status() {
        StatusCode::OK => Some(openapi::checked(Method::GET, response).await),
        StatusCode::NOT_FOUND => None,
        status => panic!("Unexpected read list status {status}"),
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

fn id(read_list: &Value) -> &str {
    read_list["id"].as_str().expect("Read list ID")
}

fn is_utc_second(value: &Value) -> bool {
    let value = value.as_str().expect("Date time");
    jiff::civil::DateTime::strptime("%Y-%m-%dT%H:%M:%SZ", value).is_ok()
}

#[tokio::test]
async fn admins_create_read_lists_in_the_given_order() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let books = [&f.b1, &f.a2, &f.a1];
    let created = create_list(&f, &json!({"name": "Arc", "bookIds": books})).await;
    assert_eq!(created["name"], "Arc");
    assert_eq!(created["summary"], "");
    assert_eq!(created["ordered"], true);
    assert_eq!(created["bookIds"], json!(books));
    assert_eq!(created["filtered"], false);
    assert!(is_utc_second(&created["createdDate"]), "{created}");
    assert!(is_utc_second(&created["lastModifiedDate"]), "{created}");
    let stored = stored_members(&f.pool, id(&created)).await;
    assert_eq!(
        stored,
        (
            3,
            vec![(0, f.b1.clone()), (1, f.a2.clone()), (2, f.a1.clone())]
        )
    );

    let post = |body: Value| {
        let f = &f;
        async move { status(f, None, Method::POST, READ_LISTS, Some(&body)).await }
    };
    let (code, body) = post(json!({"name": "ARC", "bookIds": [&f.a1]})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Read list name already exists");
    let (code, _) = post(json!({"name": "Other", "bookIds": [&f.a1, "missing"]})).await;
    assert_eq!(
        code,
        StatusCode::BAD_REQUEST,
        "an unknown book is a client error"
    );
    let (code, _) = post(json!({"name": "Other"})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST, "bookIds are required");
    let (code, body) = post(json!({"name": " ", "bookIds": []})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let mut violations = body["violations"].as_array().expect("Violations").clone();
    violations.sort_by_key(|violation| violation["fieldName"].to_string());
    assert_eq!(
        violations,
        [
            json!({"fieldName": "bookIds", "message": "must not be empty"}),
            json!({"fieldName": "name", "message": "must not be blank"}),
        ]
    );
    let (code, body) = post(json!({"name": "Other", "bookIds": [&f.a1, &f.a1]})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["violations"],
        json!([{"fieldName": "bookIds", "message": "must only contain unique elements"}])
    );

    create_user(&f.server, "reader@example.com", json!({})).await;
    let reader = Some("reader@example.com");
    let valid = json!({"name": "Other", "bookIds": [&f.a1]});
    let (code, _) = status(&f, reader, Method::POST, READ_LISTS, Some(&valid)).await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = status(
        &f,
        reader,
        Method::POST,
        READ_LISTS,
        Some(&json!({"name": ""})),
    )
    .await;
    assert_eq!(
        code,
        StatusCode::BAD_REQUEST,
        "the body is read before the role check"
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM READLIST")
        .fetch_one(&f.pool)
        .await
        .expect("Read lists");
    assert_eq!(count, 1, "no rejected request creates a read list");
    f.server.stop().await;
}

#[tokio::test]
async fn users_see_only_the_books_they_may_access() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let created = create_list(
        &f,
        &json!({"name": "Arc", "bookIds": [&f.b1, &f.a2, &f.a1]}),
    )
    .await;
    let id = id(&created);
    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.com", library_a).await;
    let no_library = json!({"sharedLibraries": {"all": false, "libraryIds": []}});
    create_user(&f.server, "none@example.com", no_library).await;
    let allow_12 = json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}});
    create_user(&f.server, "twelve@example.com", allow_12).await;
    rate(&f.server, &f.series_a, 18).await;
    rate(&f.server, &f.series_b, 6).await;

    let seen = get_list(&f, Some("a@example.com"), id)
        .await
        .expect("Visible");
    assert_eq!(seen["bookIds"], json!([&f.a2, &f.a1]));
    assert_eq!(seen["filtered"], true);
    let seen = get_list(&f, Some("twelve@example.com"), id)
        .await
        .expect("Visible");
    assert_eq!(seen["bookIds"], json!([&f.b1]));
    assert_eq!(seen["filtered"], true);
    assert_eq!(get_list(&f, Some("none@example.com"), id).await, None);
    let seen = get_list(&f, None, id).await.expect("Visible");
    assert_eq!(seen, created, "the admin sees every book");
    assert_eq!(get_list(&f, None, "missing").await, None);
    f.server.stop().await;
}

#[tokio::test]
async fn updates_replace_every_member_or_none() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let created = create_list(&f, &json!({"name": "X", "bookIds": [&f.a1, &f.a2]})).await;
    create_list(&f, &json!({"name": "Y", "bookIds": [&f.b1]})).await;
    let path = format!("{READ_LISTS}/{}", id(&created));
    let patch = |email: Option<&'static str>, body: Value| {
        let path = path.clone();
        let f = &f;
        async move { status(f, email, Method::PATCH, &path, Some(&body)).await }
    };

    let (code, body) = patch(None, json!({"name": "y"})).await;
    assert_eq!(
        (code, &body["message"]),
        (
            StatusCode::BAD_REQUEST,
            &json!("Read list name already exists")
        )
    );
    let (code, _) = patch(None, json!({"bookIds": [&f.a2, &f.b1, "missing"]})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let (code, body) = patch(None, json!({"name": "", "bookIds": []})).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["violations"].as_array().map(Vec::len),
        Some(2),
        "{body}"
    );
    let unchanged = (2, vec![(0, f.a1.clone()), (1, f.a2.clone())]);
    assert_eq!(stored_members(&f.pool, id(&created)).await, unchanged);

    let body = json!({"name": "x", "summary": "S", "ordered": false, "bookIds": [&f.b1, &f.a1]});
    assert_eq!(
        patch(None, body).await.0,
        StatusCode::NO_CONTENT,
        "recasing its own name is allowed"
    );
    let updated = get_list(&f, None, id(&created)).await.expect("Updated");
    assert_eq!(
        (
            &updated["name"],
            &updated["summary"],
            &updated["ordered"],
            &updated["bookIds"]
        ),
        (
            &json!("x"),
            &json!("S"),
            &json!(false),
            &json!([&f.b1, &f.a1])
        )
    );
    assert!(updated["lastModifiedDate"].as_str() >= created["lastModifiedDate"].as_str());
    let replaced = (2, vec![(0, f.b1.clone()), (1, f.a1.clone())]);
    assert_eq!(stored_members(&f.pool, id(&created)).await, replaced);

    // An admin who sees only `b1` changes the summary without dropping `a1`.
    rate(&f.server, &f.series_a, 18).await;
    rate(&f.server, &f.series_b, 6).await;
    let restricted =
        json!({"roles": ["ADMIN"], "ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}});
    create_user(&f.server, "restricted@example.com", restricted).await;
    assert_eq!(
        patch(Some("restricted@example.com"), json!({"summary": "T"}))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(stored_members(&f.pool, id(&created)).await, replaced);

    create_user(&f.server, "reader@example.com", json!({})).await;
    assert_eq!(
        patch(Some("reader@example.com"), json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    let (code, _) = status(
        &f,
        None,
        Method::PATCH,
        &format!("{READ_LISTS}/missing"),
        Some(&json!({})),
    )
    .await;
    assert_eq!(code, StatusCode::NOT_FOUND);
    f.server.stop().await;
}

#[tokio::test]
async fn deleting_a_read_list_removes_its_members() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let created = create_list(&f, &json!({"name": "X", "bookIds": [&f.a1, &f.b1]})).await;
    let path = format!("{READ_LISTS}/{}", id(&created));
    create_user(&f.server, "reader@example.com", json!({})).await;

    let reader = Some("reader@example.com");
    assert_eq!(
        status(&f, reader, Method::DELETE, &path, None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        status(&f, None, Method::DELETE, &path, None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(get_list(&f, None, id(&created)).await, None);
    let members: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM READLIST_BOOK")
        .fetch_one(&f.pool)
        .await
        .expect("Members");
    assert_eq!(members, 0);
    assert_eq!(
        status(&f, None, Method::DELETE, &path, None).await.0,
        StatusCode::NOT_FOUND
    );
    f.server.stop().await;
}
