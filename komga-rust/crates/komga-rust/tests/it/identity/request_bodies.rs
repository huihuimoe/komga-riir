use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde_json::{Value, json};

use crate::support::{TestServer, json};

const READER_EMAIL: &str = "reader@example.com";
const READER_PASSWORD: &str = "readerpass";
const JSON_TYPES: &str = "application/json, application/*+json";

#[tokio::test]
async fn unparseable_or_missing_bodies_follow_spring_message_conversion() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let users = "/api/v2/users";
    let reader_id = create_reader(&server).await;
    let reader = format!("{users}/{reader_id}");

    let parse_errors = [
        (Method::POST, users, "{"),
        // Missing non-null Kotlin property.
        (Method::POST, users, r#"{"email": "new@example.com"}"#),
        (
            Method::POST,
            users,
            r#"{"email": "new@example.com", "password": 1}"#,
        ),
        (
            Method::PATCH,
            reader.as_str(),
            r#"{"ageRestriction": {"age": 1, "restriction": "SOME"}}"#,
        ),
        // Property names and enum values are case-sensitive, unlike Kotlin's mapper.
        (
            Method::POST,
            users,
            r#"{"Email": "new@example.com", "password": "secret"}"#,
        ),
        (
            Method::PATCH,
            reader.as_str(),
            r#"{"ageRestriction": {"age": 1, "restriction": "exclude"}}"#,
        ),
        (Method::POST, users, "  "),
    ];
    for (method, path, body) in parse_errors {
        let response = server
            .admin(method, path)
            .header(CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .expect("Unparseable body request failed");
        let message = assert_spring_error(response, StatusCode::BAD_REQUEST, path).await;
        assert!(
            message.starts_with("JSON parse error: "),
            "{body:?} must fail in Jackson, got {message:?}"
        );
    }

    let missing = [
        server
            .admin(Method::POST, users)
            .header(CONTENT_TYPE, "application/json"),
        server.admin(Method::POST, users),
    ];
    for request in missing {
        let response = request.send().await.expect("Empty body request failed");
        let message = assert_spring_error(response, StatusCode::BAD_REQUEST, users).await;
        assert_eq!(message, "Required request body is missing");
    }
    server.stop().await;
}

#[tokio::test]
async fn unsupported_content_types_follow_spring_message_conversion() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let users = "/api/v2/users";
    let reader = format!("{users}/{}", create_reader(&server).await);

    let unsupported = [
        (
            server
                .admin(Method::POST, users)
                .header(CONTENT_TYPE, "Text/Plain; charset=UTF-8")
                .body("{}"),
            users,
            "Content-Type 'text/plain;charset=UTF-8' is not supported.",
            false,
        ),
        (
            server.admin(Method::POST, users).body("{}"),
            users,
            "Content-Type 'application/octet-stream' is not supported.",
            false,
        ),
        (
            server
                .admin(Method::PATCH, &reader)
                .header(CONTENT_TYPE, "text/csv")
                .body("labelsAllow"),
            reader.as_str(),
            "Content-Type 'text/csv' is not supported.",
            true,
        ),
        (
            server
                .admin(Method::POST, users)
                .header(CONTENT_TYPE, "json"),
            users,
            "Could not parse Content-Type.",
            false,
        ),
    ];
    for (request, path, detail, patch) in unsupported {
        let response = request
            .send()
            .await
            .expect("Unsupported body request failed");
        let headers = response.headers().clone();
        let message = assert_spring_error(response, StatusCode::UNSUPPORTED_MEDIA_TYPE, path).await;
        assert_eq!(message, detail);
        assert_eq!(headers[ACCEPT], JSON_TYPES);
        assert_eq!(headers.get("accept-patch").is_some(), patch, "{detail}");
    }

    // A `+json` subtype is read by the JSON converter.
    let response = server
        .admin(Method::PATCH, &reader)
        .header(CONTENT_TYPE, "application/merge-patch+json")
        .body(r#"{"labelsAllow": ["kids"]}"#)
        .send()
        .await
        .expect("JSON subtype request failed");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    server.stop().await;
}

#[tokio::test]
async fn request_bodies_are_validated_before_method_security() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    create_reader(&server).await;
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
    let reader = |method: Method, path: &str| {
        server
            .request(method, path)
            .basic_auth(READER_EMAIL, Some(READER_PASSWORD))
    };

    let users = "/api/v2/users";
    let admin_self = format!("{users}/{admin_id}");
    let admin_password = format!("{users}/{admin_id}/password");
    let checks: [(RequestBuilder, &str, Value, Value); 3] = [
        (
            reader(Method::POST, users),
            users,
            json!({"email": "new@example.com", "password": " "}),
            json!({"email": "new@example.com", "password": "secret"}),
        ),
        (
            server.admin(Method::PATCH, &admin_self),
            &admin_self,
            json!({"ageRestriction": {"age": -1, "restriction": "EXCLUDE"}}),
            json!({"roles": []}),
        ),
        (
            reader(Method::PATCH, &admin_password),
            &admin_password,
            json!({"password": ""}),
            json!({"password": "stolen"}),
        ),
    ];
    for (request, path, invalid, valid) in checks {
        let second = request.try_clone().expect("Request must be cloneable");
        let response = request
            .json(&invalid)
            .send()
            .await
            .expect("Invalid body request failed");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        assert!(json(response).await.get("violations").is_some(), "{path}");

        let response = second
            .json(&valid)
            .send()
            .await
            .expect("Forbidden request failed");
        let message = assert_spring_error(response, StatusCode::FORBIDDEN, path).await;
        assert_eq!(message, "Forbidden");
    }

    // Authentication still precedes body binding.
    let response = server
        .request(Method::POST, users)
        .body("{")
        .send()
        .await
        .expect("Anonymous request failed");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    server.stop().await;
}

async fn create_reader(server: &TestServer) -> String {
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&json!({"email": READER_EMAIL, "password": READER_PASSWORD}))
        .send()
        .await
        .expect("Reader creation failed");
    assert_eq!(response.status(), StatusCode::CREATED);
    json(response).await["id"]
        .as_str()
        .expect("Reader ID")
        .to_owned()
}

/// Asserts a Spring Boot default error body and returns its message.
async fn assert_spring_error(
    response: reqwest::Response,
    status: StatusCode,
    path: &str,
) -> String {
    assert_eq!(response.status(), status, "{path}");
    let body = json(response).await;
    assert_eq!(body["status"], status.as_u16());
    assert_eq!(
        body["error"],
        status.canonical_reason().expect("Standard status")
    );
    assert_eq!(body["path"], path);
    assert!(
        body["timestamp"].is_string(),
        "Error must carry a timestamp"
    );
    body["message"]
        .as_str()
        .expect("Error message must be a string")
        .to_owned()
}
