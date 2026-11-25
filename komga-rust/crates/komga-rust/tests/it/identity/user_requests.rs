use std::collections::BTreeSet;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::{TestServer, json, openapi};

#[tokio::test]
async fn user_roles_follow_baseline_parsing_and_dto_output() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;

    let me = json(
        server
            .admin(Method::GET, "/api/v2/users/me")
            .send()
            .await
            .expect("Current-user request failed"),
    )
    .await;
    assert_eq!(
        me["roles"],
        json!([
            "ADMIN",
            "FILE_DOWNLOAD",
            "PAGE_STREAMING",
            "KOBO_SYNC",
            "KOREADER_SYNC",
            "USER"
        ])
    );

    let without_roles = create_user(
        &server,
        json!({"email": "plain@example.com", "password": "secret"}),
    )
    .await;
    assert_eq!(without_roles["roles"], json!(["USER"]));

    let mixed_roles = create_user(
        &server,
        json!({
            "email": "mixed@example.com",
            "password": "secret",
            "roles": ["admin", "FILE_DOWNLOAD", "FILE_DOWNLOAD", "UNKNOWN"]
        }),
    )
    .await;
    assert_eq!(mixed_roles["roles"], json!(["FILE_DOWNLOAD", "USER"]));

    let response = server
        .admin(
            Method::PATCH,
            &format!(
                "/api/v2/users/{}",
                without_roles["id"].as_str().expect("User ID")
            ),
        )
        .json(&json!({"roles": ["KOBO_SYNC", "Kobo_Sync"]}))
        .send()
        .await
        .expect("Role update request failed");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let users = json(
        server
            .admin(Method::GET, "/api/v2/users")
            .send()
            .await
            .expect("User-list request failed"),
    )
    .await;
    let updated = users
        .as_array()
        .expect("User list must be an array")
        .iter()
        .find(|user| user["email"] == "plain@example.com")
        .expect("Updated user must be listed");
    assert_eq!(updated["roles"], json!(["KOBO_SYNC", "USER"]));

    server.stop().await;
}

#[tokio::test]
async fn invalid_request_bodies_return_validation_error_response() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;

    for (email, password) in [("reader", " \t"), ("reader@localhost", ""), ("", "\u{1f}")] {
        let response = server
            .admin(Method::POST, "/api/v2/users")
            .json(&json!({"email": email, "password": password}))
            .send()
            .await
            .expect("Invalid user creation request failed");
        assert_violations(
            Method::POST,
            response,
            &[
                ("email", "must be a well-formed email address"),
                ("password", "must not be blank"),
            ],
        )
        .await;
    }

    let admin_id = json(
        server
            .admin(Method::GET, "/api/v2/users/me")
            .send()
            .await
            .expect("Current-user request failed"),
    )
    .await["id"]
        .as_str()
        .expect("Admin ID")
        .to_owned();
    for path in [
        "/api/v2/users/me/password".to_owned(),
        format!("/api/v2/users/{admin_id}/password"),
    ] {
        let response = server
            .admin(Method::PATCH, &path)
            .json(&json!({"password": "  "}))
            .send()
            .await
            .expect("Blank password request failed");
        assert_violations(
            Method::PATCH,
            response,
            &[("password", "must not be blank")],
        )
        .await;
    }

    let response = server
        .admin(Method::POST, "/api/v2/users/me/api-keys")
        .json(&json!({"comment": "\n"}))
        .send()
        .await
        .expect("Blank API-key comment request failed");
    assert_violations(Method::POST, response, &[("comment", "must not be blank")]).await;

    // Java `trim()` keeps non-ASCII whitespace, so this password is not blank.
    let created = create_user(
        &server,
        json!({"email": "ideographic@example.com", "password": "\u{3000}"}),
    )
    .await;
    assert_eq!(created["email"], "ideographic@example.com");

    server.stop().await;
}

async fn create_user(server: &TestServer, body: Value) -> Value {
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&body)
        .send()
        .await
        .expect("User creation request failed");
    assert_eq!(response.status(), StatusCode::CREATED);
    openapi::checked(Method::POST, response).await
}

async fn assert_violations(method: Method, response: reqwest::Response, expected: &[(&str, &str)]) {
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = openapi::checked(method, response).await;
    let object = body
        .as_object()
        .expect("Validation error must be an object");
    assert_eq!(object.keys().collect::<Vec<_>>(), ["violations"]);
    // Bean Validation does not order violations.
    let actual = body["violations"]
        .as_array()
        .expect("Violations must be an array")
        .iter()
        .map(|violation| {
            (
                violation["fieldName"]
                    .as_str()
                    .expect("fieldName")
                    .to_owned(),
                violation["message"].as_str().expect("message").to_owned(),
            )
        })
        .collect::<BTreeSet<_>>();
    let expected = expected
        .iter()
        .map(|(field, message)| ((*field).to_owned(), (*message).to_owned()))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}
