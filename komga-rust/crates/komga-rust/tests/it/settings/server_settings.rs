use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::support::{TestServer, db, json, openapi};

const READER_EMAIL: &str = "reader@example.com";
const READER_PASSWORD: &str = "reader-password";

#[tokio::test]
async fn settings_round_trip_and_survive_restart() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let port = server.port();

    let defaults = get_settings(&server).await;
    assert_eq!(
        defaults,
        json!({
            "deleteEmptyCollections": true,
            "deleteEmptyReadLists": true,
            "rememberMeDurationDays": 365,
            "thumbnailSize": "DEFAULT",
            "taskPoolSize": 1,
            "serverPort": {"configurationSource": 25600, "databaseSource": null, "effectiveValue": port},
            "serverContextPath": {"configurationSource": null, "databaseSource": null, "effectiveValue": ""},
            "koboProxy": false,
            "kepubifyPath": {"configurationSource": null, "databaseSource": null, "effectiveValue": null},
            "maxUploadFileSizeBytes": 1_048_576,
            "nextUiAsDefault": false
        }),
        "Fresh database values come from the baseline migration and provider defaults"
    );

    let updated = update_every_setting(&server, port).await;

    server.stop().await;
    // The stored context path applies from now on.
    let server = TestServer::start(temp_dir.path()).await;
    let mut restarted = get_settings_at(&server, "/komga").await;
    restarted["serverPort"]["effectiveValue"] = updated["serverPort"]["effectiveValue"].clone();
    let mut expected = updated;
    expected["serverContextPath"]["effectiveValue"] = json!("/komga");
    assert_eq!(
        restarted, expected,
        "Stored settings must not drift across a restart"
    );

    let response = server
        .admin(Method::PATCH, "/komga/api/v1/settings")
        .json(&json!({
            "serverPort": null,
            "serverContextPath": null,
            "koboPort": null,
            "kepubifyPath": null,
            "koboProxy": null
        }))
        .send()
        .await
        .expect("Settings reset failed");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let reset = get_settings_at(&server, "/komga").await;
    assert_eq!(reset["serverPort"]["databaseSource"], Value::Null);
    assert_eq!(reset["serverContextPath"]["databaseSource"], Value::Null);
    assert!(
        reset.get("koboPort").is_none(),
        "A null Kobo port is omitted"
    );
    assert_eq!(reset["kepubifyPath"]["databaseSource"], Value::Null);
    assert_eq!(
        reset["koboProxy"], true,
        "null leaves non-nullable settings unchanged"
    );
    server.stop().await;
}

/// Updates every writable setting and checks each stored and effective value.
async fn update_every_setting(server: &TestServer, port: u16) -> Value {
    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({
            "deleteEmptyCollections": false,
            "rememberMeDurationDays": 30,
            "thumbnailSize": "XLARGE",
            "taskPoolSize": 4,
            "serverPort": 8080,
            "serverContextPath": "/komga",
            "koboProxy": true,
            "koboPort": 443,
            "kepubifyPath": "/usr/bin/kepubify",
            "nextUiAsDefault": true
        }))
        .send()
        .await
        .expect("Settings update failed");
    assert_eq!(
        openapi::checked(Method::PATCH, response).await,
        Value::Null,
        "PATCH returns 204 without a body"
    );

    let updated = get_settings(server).await;
    assert_eq!(updated["deleteEmptyCollections"], false);
    assert_eq!(
        updated["deleteEmptyReadLists"], true,
        "Omitted fields keep their value"
    );
    assert_eq!(updated["rememberMeDurationDays"], 30);
    assert_eq!(updated["thumbnailSize"], "XLARGE");
    assert_eq!(updated["taskPoolSize"], 4);
    assert_eq!(
        updated["serverPort"],
        json!({"configurationSource": 25600, "databaseSource": 8080, "effectiveValue": port}),
        "A stored port only applies after a restart"
    );
    assert_eq!(
        updated["serverContextPath"],
        json!({"configurationSource": null, "databaseSource": "/komga", "effectiveValue": ""})
    );
    assert_eq!(updated["koboProxy"], true);
    assert_eq!(updated["koboPort"], 443);
    assert_eq!(
        updated["kepubifyPath"],
        json!({"configurationSource": null, "databaseSource": "/usr/bin/kepubify", "effectiveValue": "/usr/bin/kepubify"})
    );
    assert_eq!(updated["nextUiAsDefault"], true);

    updated
}

#[tokio::test]
async fn invalid_or_failed_updates_keep_effective_settings() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let before = get_settings(&server).await;

    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({
            "deleteEmptyCollections": false,
            "rememberMeDurationDays": 0,
            "taskPoolSize": -1,
            "serverPort": 65536,
            "serverContextPath": "/komga/",
            "koboPort": 0
        }))
        .send()
        .await
        .expect("Invalid settings update failed");
    let body = openapi::checked(Method::PATCH, response).await;
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
    assert_eq!(
        violations,
        [
            ("koboPort", "must be greater than 0"),
            ("rememberMeDurationDays", "must be greater than 0"),
            ("serverContextPath", r#"must match "^/[\w-/]*[a-zA-Z0-9]$""#),
            ("serverPort", "must be less than or equal to 65535"),
            ("taskPoolSize", "must be greater than 0"),
        ]
        .map(|(field, message)| (field.to_owned(), message.to_owned()))
    );
    assert_eq!(
        get_settings(&server).await,
        before,
        "A rejected update changes nothing"
    );

    // A write failure midway must roll back the earlier writes of the same update.
    let pool = db::main_pool(temp_dir.path()).await;
    db::inject_failure(
        &pool,
        "BEFORE INSERT ON SERVER_SETTINGS WHEN NEW.KEY = 'NEXTUI_AS_DEFAULT'",
    )
    .await;
    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({"deleteEmptyCollections": false, "nextUiAsDefault": true}))
        .send()
        .await
        .expect("Failing settings update failed");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        get_settings(&server).await,
        before,
        "A failed update changes nothing"
    );
    pool.close().await;
    server.stop().await;
}

#[tokio::test]
async fn non_admins_see_only_public_settings_and_cannot_update() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&json!({"email": READER_EMAIL, "password": READER_PASSWORD}))
        .send()
        .await
        .expect("User creation failed");
    assert_eq!(response.status(), StatusCode::CREATED);

    let reader = |method| {
        server
            .request(method, "/api/v1/settings")
            .basic_auth(READER_EMAIL, Some(READER_PASSWORD))
    };
    let response = reader(Method::GET).send().await.expect("Reader GET failed");
    assert_eq!(
        openapi::checked(Method::GET, response).await,
        json!({"maxUploadFileSizeBytes": 1_048_576})
    );

    let response = reader(Method::PATCH)
        .json(&json!({"nextUiAsDefault": true}))
        .send()
        .await
        .expect("Reader PATCH failed");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = reader(Method::PATCH)
        .json(&json!({"taskPoolSize": 0}))
        .send()
        .await
        .expect("Reader invalid PATCH failed");
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Validation runs before method security"
    );
    assert_eq!(get_settings(&server).await["nextUiAsDefault"], false);

    let response = server
        .request(Method::GET, "/api/v1/settings")
        .send()
        .await
        .expect("Anonymous GET failed");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    server.stop().await;
}

#[tokio::test]
async fn renewed_remember_me_key_is_stored_for_the_next_start() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let pool = db::main_pool(temp_dir.path()).await;
    let before = remember_me_key(&pool).await;

    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({"renewRememberMeKey": false}))
        .send()
        .await
        .expect("No-op renew failed");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(remember_me_key(&pool).await, before);

    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&json!({"renewRememberMeKey": true}))
        .send()
        .await
        .expect("Renew failed");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let renewed = remember_me_key(&pool).await;
    assert_ne!(renewed, before);
    assert_eq!(renewed.len(), 64);
    pool.close().await;
    server.stop().await;
}

async fn get_settings(server: &TestServer) -> Value {
    get_settings_at(server, "").await
}

async fn get_settings_at(server: &TestServer, context_path: &str) -> Value {
    let response = server
        .admin(Method::GET, &format!("{context_path}/api/v1/settings"))
        .send()
        .await
        .expect("Settings request failed");
    assert_eq!(response.status(), StatusCode::OK);
    json(response).await
}

async fn remember_me_key(pool: &SqlitePool) -> String {
    db::server_setting(pool, "REMEMBER_ME_KEY")
        .await
        .expect("Remember-me key must be stored")
}
