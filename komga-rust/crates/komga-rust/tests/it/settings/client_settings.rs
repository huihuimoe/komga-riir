use reqwest::header::SET_COOKIE;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::{ADMIN_EMAIL, ADMIN_PASSWORD, TestServer, db, json, openapi};

const READER_EMAIL: &str = "reader@example.com";
const READER_PASSWORD: &str = "reader-password";
const GLOBAL: &str = "/api/v1/client-settings/global";
const GLOBAL_LIST: &str = "/api/v1/client-settings/global/list";
const USER: &str = "/api/v1/client-settings/user";
const USER_LIST: &str = "/api/v1/client-settings/user/list";
const KEY_MESSAGE: &str =
    r#"must match "^[a-z](?:[a-z0-9_-]*[a-z0-9])*(?:\.[a-z0-9](?:[a-z0-9_-]*[a-z0-9])*)*$""#;

/// A claimed server with the reader account; keep the directory alive while it runs.
async fn server_with_reader() -> (komga_test_support::TestDir, TestServer) {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    create_reader(&server).await;
    (temp_dir, server)
}

#[tokio::test]
async fn global_settings_visibility_updates_and_restart() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    create_reader(&server).await;

    let response = server
        .admin(Method::PATCH, GLOBAL)
        .json(&json!({
            "app.public": {"value": "shown", "allowUnauthorized": true},
            "app.private": {"value": "{\"json\":\"object\"}", "allowUnauthorized": false}
        }))
        .send()
        .await
        .expect("Global PATCH failed");
    assert_eq!(openapi::checked(Method::PATCH, response).await, Value::Null);

    let all = json!({
        "app.public": {"value": "shown", "allowUnauthorized": true},
        "app.private": {"value": "{\"json\":\"object\"}", "allowUnauthorized": false}
    });
    assert_eq!(global_list(&server, Viewer::Admin).await, all);
    assert_eq!(global_list(&server, Viewer::Reader).await, all);
    assert_eq!(
        global_list(&server, Viewer::Anonymous).await,
        json!({"app.public": {"value": "shown", "allowUnauthorized": true}}),
        "Anonymous requests only see settings allowing unauthorized access"
    );

    // Unlike the baseline, which only replaced the value, re-saving also updates the flag.
    let response = server
        .admin(Method::PATCH, GLOBAL)
        .json(&json!({"app.public": {"value": "changed", "allowUnauthorized": false}}))
        .send()
        .await
        .expect("Global re-PATCH failed");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(global_list(&server, Viewer::Anonymous).await, json!({}));

    server.stop().await;
    let server = TestServer::start(temp_dir.path()).await;
    assert_eq!(
        global_list(&server, Viewer::Admin).await,
        json!({
            "app.public": {"value": "changed", "allowUnauthorized": false},
            "app.private": {"value": "{\"json\":\"object\"}", "allowUnauthorized": false}
        }),
        "Global settings survive a restart"
    );

    let response = server
        .admin(Method::DELETE, GLOBAL)
        .json(&json!(["app.public", "app.missing", null]))
        .send()
        .await
        .expect("Global DELETE failed");
    assert_eq!(
        openapi::checked(Method::DELETE, response).await,
        Value::Null
    );
    assert_eq!(
        global_list(&server, Viewer::Admin).await,
        json!({"app.private": {"value": "{\"json\":\"object\"}", "allowUnauthorized": false}})
    );
    server.stop().await;
}

#[tokio::test]
async fn global_settings_reject_unauthorized_writers_in_spring_order() {
    let (_temp_dir, server) = server_with_reader().await;
    let valid = json!({"setting": {"value": "value", "allowUnauthorized": false}});
    let invalid = json!({"UPPER": {"value": " ", "allowUnauthorized": false}, "ok": null});

    for method in [Method::PATCH, Method::DELETE] {
        let response = server
            .request(method.clone(), GLOBAL)
            .json(&valid)
            .send()
            .await
            .expect("Anonymous write failed");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method}");
    }
    for body in [&valid, &invalid] {
        let response = server
            .request(Method::PATCH, GLOBAL)
            .basic_auth(READER_EMAIL, Some(READER_PASSWORD))
            .json(body)
            .send()
            .await
            .expect("Reader PATCH failed");
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "Method security runs before key/value validation: {body}"
        );
    }
    let response = server
        .request(Method::DELETE, GLOBAL)
        .basic_auth(READER_EMAIL, Some(READER_PASSWORD))
        .json(&json!(["UPPER"]))
        .send()
        .await
        .expect("Reader DELETE failed");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = server
        .request(Method::PATCH, GLOBAL)
        .basic_auth(READER_EMAIL, Some(READER_PASSWORD))
        .json(&json!({"setting": {"value": null, "allowUnauthorized": false}}))
        .send()
        .await
        .expect("Reader unreadable PATCH failed");
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Body conversion fails before method security"
    );
    server.stop().await;
}

#[tokio::test]
async fn global_settings_reject_invalid_bodies_without_storing() {
    let (_temp_dir, server) = server_with_reader().await;
    let invalid = json!({"UPPER": {"value": " ", "allowUnauthorized": false}, "ok": null});
    let response = server
        .admin(Method::PATCH, GLOBAL)
        .json(&invalid)
        .send()
        .await
        .expect("Admin invalid PATCH failed");
    assert_eq!(
        violations(response).await,
        [
            (
                "saveGlobalSetting.newSettings<K>[UPPER].<map key>",
                KEY_MESSAGE
            ),
            (
                "saveGlobalSetting.newSettings[UPPER].value",
                "must not be blank"
            ),
            (
                "saveGlobalSetting.newSettings[ok].<map value>",
                "must not be null"
            ),
        ]
        .map(owned)
    );
    let response = server
        .admin(Method::DELETE, GLOBAL)
        .json(&json!(["two..dots", "fine"]))
        .send()
        .await
        .expect("Admin invalid DELETE failed");
    assert_eq!(
        violations(response).await,
        [(
            "deleteGlobalSettings.keysToDelete[].<iterable element>",
            KEY_MESSAGE
        )]
        .map(owned)
    );
    for body in [
        json!({"setting": {"value": "1", "allowUnauthorized": null}}),
        json!({"setting": {"value": "1"}}),
        json!({"setting": {"value": null, "allowUnauthorized": false}}),
    ] {
        let response = server
            .admin(Method::PATCH, GLOBAL)
            .json(&body)
            .send()
            .await
            .expect("Admin unreadable PATCH failed");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    assert_eq!(
        global_list(&server, Viewer::Admin).await,
        json!({}),
        "Rejected requests store nothing"
    );
    server.stop().await;
}

#[tokio::test]
async fn global_settings_roll_back_a_failed_write() {
    let (temp_dir, server) = server_with_reader().await;
    // A write failure midway must roll back the earlier writes of the same request.
    let pool = db::main_pool(temp_dir.path()).await;
    db::inject_failure(
        &pool,
        "BEFORE INSERT ON CLIENT_SETTINGS_GLOBAL WHEN NEW.KEY = 'b'",
    )
    .await;
    let response = server
        .admin(Method::PATCH, GLOBAL)
        .json(&json!({
            "a": {"value": "1", "allowUnauthorized": true},
            "b": {"value": "2", "allowUnauthorized": true}
        }))
        .send()
        .await
        .expect("Failing PATCH failed");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(global_list(&server, Viewer::Admin).await, json!({}));
    pool.close().await;
    server.stop().await;
}

#[tokio::test]
async fn user_settings_are_isolated_per_user() {
    let (temp_dir, server) = server_with_reader().await;
    let reader = |method| {
        server
            .request(method, USER)
            .basic_auth(READER_EMAIL, Some(READER_PASSWORD))
    };

    let response = server
        .admin(Method::PATCH, USER)
        .json(&json!({"app.theme": {"value": "dark"}, "app.size": {"value": "2"}}))
        .send()
        .await
        .expect("Admin user PATCH failed");
    assert_eq!(openapi::checked(Method::PATCH, response).await, Value::Null);
    let response = reader(Method::PATCH)
        .json(&json!({"app.theme": {"value": "light"}}))
        .send()
        .await
        .expect("Reader user PATCH failed");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    assert_eq!(
        user_list(&server, Viewer::Admin).await,
        json!({"app.theme": {"value": "dark"}, "app.size": {"value": "2"}})
    );
    assert_eq!(
        user_list(&server, Viewer::Reader).await,
        json!({"app.theme": {"value": "light"}})
    );
    assert_eq!(
        global_list(&server, Viewer::Admin).await,
        json!({}),
        "User settings are not global settings"
    );

    let response = reader(Method::DELETE)
        .json(&json!(["app.theme", "app.size"]))
        .send()
        .await
        .expect("Reader user DELETE failed");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(user_list(&server, Viewer::Reader).await, json!({}));
    assert_eq!(
        user_list(&server, Viewer::Admin).await,
        json!({"app.theme": {"value": "dark"}, "app.size": {"value": "2"}}),
        "Deleting only affects the caller's settings"
    );

    server.stop().await;
    let server = TestServer::start(temp_dir.path()).await;
    assert_eq!(
        user_list(&server, Viewer::Admin).await,
        json!({"app.theme": {"value": "dark"}, "app.size": {"value": "2"}}),
        "User settings survive a restart"
    );
    server.stop().await;
}

#[tokio::test]
async fn user_settings_validate_keys_and_require_authentication() {
    let (_temp_dir, server) = server_with_reader().await;
    let reader = |method| {
        server
            .request(method, USER)
            .basic_auth(READER_EMAIL, Some(READER_PASSWORD))
    };

    let response = reader(Method::PATCH)
        .json(&json!({".bad": {"value": ""}, "app.empty": null}))
        .send()
        .await
        .expect("Reader invalid PATCH failed");
    assert_eq!(
        violations(response).await,
        [
            (
                "saveUserSetting.newSettings<K>[.bad].<map key>",
                KEY_MESSAGE
            ),
            (
                "saveUserSetting.newSettings[.bad].value",
                "must not be blank"
            ),
            (
                "saveUserSetting.newSettings[app.empty].<map value>",
                "must not be null"
            ),
        ]
        .map(owned)
    );
    let response = reader(Method::DELETE)
        .json(&json!(["end."]))
        .send()
        .await
        .expect("Reader invalid DELETE failed");
    assert_eq!(
        violations(response).await,
        [(
            "deleteUserSettings.keysToDelete[].<iterable element>",
            KEY_MESSAGE
        )]
        .map(owned)
    );

    for (method, path) in [(Method::PATCH, USER), (Method::GET, USER_LIST)] {
        let response = server
            .request(method, path)
            .json(&json!({}))
            .send()
            .await
            .expect("Anonymous user settings request failed");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }

    server.stop().await;
}

#[tokio::test]
async fn anonymous_global_list_still_checks_sent_credentials() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;

    let response = server
        .request(Method::GET, GLOBAL_LIST)
        .basic_auth("admin@example.com", Some("wrong"))
        .send()
        .await
        .expect("Bad Basic request failed");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "Failed Basic credentials are rejected even where anonymous access is permitted"
    );
    let response = server
        .request(Method::GET, GLOBAL_LIST)
        .header("X-API-Key", "0123456789abcdef0123456789abcdef")
        .send()
        .await
        .expect("Bad API key request failed");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = server
        .request(Method::GET, GLOBAL_LIST)
        .header("Cookie", "komga-remember-me=forged; KOMGA-SESSION=unknown")
        .send()
        .await
        .expect("Bad cookie request failed");
    assert_eq!(response.status(), StatusCode::OK);
    let cookies: Vec<&str> = response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .map(|value| value.to_str().expect("ASCII cookie"))
        .collect();
    assert!(
        cookies
            .iter()
            .any(|cookie| cookie.starts_with("komga-remember-me=;") && cookie.contains("Max-Age=0")),
        "A rejected remember-me cookie is cleared while continuing anonymously: {cookies:?}"
    );
    assert_eq!(openapi::checked(Method::GET, response).await, json!({}));

    let response = server
        .request(Method::GET, GLOBAL_LIST)
        .basic_auth(ADMIN_EMAIL, Some(ADMIN_PASSWORD))
        .send()
        .await
        .expect("Admin request failed");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .any(|value| value
                .to_str()
                .is_ok_and(|cookie| cookie.starts_with("KOMGA-SESSION="))),
        "Basic authentication creates a session on permitted routes too"
    );
    server.stop().await;
}

#[derive(Clone, Copy)]
enum Viewer {
    Admin,
    Reader,
    Anonymous,
}

fn request(server: &TestServer, viewer: Viewer, path: &str) -> reqwest::RequestBuilder {
    match viewer {
        Viewer::Admin => server.admin(Method::GET, path),
        Viewer::Reader => server
            .request(Method::GET, path)
            .basic_auth(READER_EMAIL, Some(READER_PASSWORD)),
        Viewer::Anonymous => server.request(Method::GET, path),
    }
}

async fn global_list(server: &TestServer, viewer: Viewer) -> Value {
    let response = request(server, viewer, GLOBAL_LIST)
        .send()
        .await
        .expect("Global list request failed");
    openapi::checked(Method::GET, response).await
}

async fn user_list(server: &TestServer, viewer: Viewer) -> Value {
    let response = request(server, viewer, USER_LIST)
        .send()
        .await
        .expect("User list request failed");
    openapi::checked(Method::GET, response).await
}

async fn create_reader(server: &TestServer) {
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&json!({"email": READER_EMAIL, "password": READER_PASSWORD}))
        .send()
        .await
        .expect("User creation failed");
    assert_eq!(response.status(), StatusCode::CREATED);
}

async fn violations(response: reqwest::Response) -> Vec<(String, String)> {
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = json(response).await;
    let mut violations: Vec<(String, String)> = body["violations"]
        .as_array()
        .expect("Validation errors list violations")
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
        .collect();
    violations.sort();
    violations
}

fn owned((field, message): (&str, &str)) -> (String, String) {
    (field.to_owned(), message.to_owned())
}
