use reqwest::header::{COOKIE, SET_COOKIE};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::{TestServer, db, openapi};

const READER_EMAIL: &str = "reader@example.com";
const READER_PASSWORD: &str = "readerpass";

#[tokio::test]
async fn library_sharing_and_content_restrictions_persist_and_revoke_sessions() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    // Sharing only references library rows; seeding them skips creating root folders.
    let pool = db::main_pool(temp_dir.path()).await;
    for id in ["library-a", "library-b"] {
        db::seed::library(&pool, id).await;
    }

    let (reader, plain) = create_users(&server).await;
    let reader_id = reader["id"].as_str().expect("Reader ID").to_owned();
    let expected = assert_patch_semantics(&server, &reader).await;
    let plain_id = plain["id"].as_str().expect("ID");
    assert_rejected_patches(&server, &reader_id, plain_id, &expected).await;

    server.stop().await;
    pool.close().await;
    let server = TestServer::start(temp_dir.path()).await;
    assert_user_access(&find_user(&server, &reader_id).await, &expected);

    patch_reader(
        &server,
        &reader_id,
        json!({"sharedLibraries": {"all": true, "libraryIds": ["library-b"]}}),
    )
    .await;
    let shared_all = find_user(&server, &reader_id).await;
    assert_eq!(shared_all["sharedAllLibraries"], true);
    assert_eq!(shared_all["sharedLibrariesIds"], json!([]));
    server.stop().await;
}

/// Creates the restricted reader and an unrestricted user, checking request normalization.
async fn create_users(server: &TestServer) -> (Value, Value) {
    let reader = create_user(
        server,
        json!({
            "email": READER_EMAIL,
            "password": READER_PASSWORD,
            "sharedLibraries": {"all": false, "libraryIds": ["library-a", "missing"]},
            "ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"},
            "labelsAllow": [" Kids ", "shared", "  "],
            "labelsExclude": ["SHARED"]
        }),
    )
    .await;
    assert_eq!(reader["roles"], json!(["USER"]));
    assert_eq!(reader["sharedAllLibraries"], false);
    assert_eq!(reader["sharedLibrariesIds"], json!(["library-a"]));
    assert_eq!(reader["labelsAllow"], json!(["kids"]));
    assert_eq!(reader["labelsExclude"], json!(["shared"]));
    assert_eq!(
        reader["ageRestriction"],
        json!({"age": 12, "restriction": "ALLOW_ONLY"})
    );

    let plain = create_user(
        server,
        json!({"email": "plain@example.com", "password": "secret"}),
    )
    .await;
    assert_eq!(plain["sharedAllLibraries"], true);
    assert_eq!(plain["sharedLibrariesIds"], json!([]));
    assert_eq!(plain["labelsAllow"], json!([]));
    assert!(plain.get("ageRestriction").is_none());

    (reader, plain)
}

/// Partial updates keep absent fields; any access change revokes sessions.
async fn assert_patch_semantics(server: &TestServer, reader: &Value) -> Value {
    let reader_id = reader["id"].as_str().expect("Reader ID");
    let session = reader_session(server).await;
    assert_eq!(&reader_me(server, &session).await, reader);

    // Clearing one field keeps the others and revokes sessions.
    patch_reader(server, reader_id, json!({"labelsExclude": null})).await;
    assert_eq!(
        reader_me_status(server, &session).await,
        StatusCode::UNAUTHORIZED
    );
    let session = reader_session(server).await;
    let after_clear = reader_me(server, &session).await;
    assert_eq!(after_clear["labelsExclude"], json!([]));
    assert_eq!(after_clear["labelsAllow"], json!(["kids"]));
    assert_eq!(after_clear["sharedLibrariesIds"], json!(["library-a"]));
    assert_eq!(after_clear["ageRestriction"], reader["ageRestriction"]);

    // An empty patch changes nothing and keeps sessions.
    patch_reader(server, reader_id, json!({})).await;
    assert_eq!(reader_me(server, &session).await, after_clear);

    patch_reader(
        server,
        reader_id,
        json!({
            "ageRestriction": {"age": 16, "restriction": "NONE"},
            "sharedLibraries": {"all": false, "libraryIds": ["library-a", "library-b"]},
            "labelsAllow": ["Kids", "teen"]
        }),
    )
    .await;
    assert_eq!(
        reader_me_status(server, &session).await,
        StatusCode::UNAUTHORIZED
    );
    let expected = json!({
        "sharedAllLibraries": false,
        "sharedLibrariesIds": ["library-a", "library-b"],
        "labelsAllow": ["kids", "teen"],
        "labelsExclude": []
    });
    let updated = find_user(server, reader_id).await;
    assert_user_access(&updated, &expected);
    assert!(updated.get("ageRestriction").is_none());

    expected
}

/// Invalid or unauthorized updates leave the reader unchanged.
async fn assert_rejected_patches(
    server: &TestServer,
    reader_id: &str,
    plain_id: &str,
    expected: &Value,
) {
    let response = server
        .admin(Method::PATCH, &format!("/api/v2/users/{reader_id}"))
        .json(&json!({"ageRestriction": {"age": -1, "restriction": "EXCLUDE"}}))
        .send()
        .await
        .expect("Invalid age patch failed");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        openapi::checked(Method::PATCH, response).await,
        json!({"violations": [{"fieldName": "ageRestriction.age", "message": "must be greater than or equal to 0"}]})
    );
    let response = server
        .request(Method::PATCH, &format!("/api/v2/users/{plain_id}"))
        .basic_auth(READER_EMAIL, Some(READER_PASSWORD))
        .json(&json!({"labelsAllow": []}))
        .send()
        .await
        .expect("Non-admin patch failed");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_user_access(&find_user(server, reader_id).await, expected);
}

fn assert_user_access(user: &Value, expected: &Value) {
    for (field, value) in expected.as_object().expect("Expected fields") {
        assert_eq!(&user[field], value, "{field}");
    }
}

async fn create_user(server: &TestServer, body: Value) -> Value {
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&body)
        .send()
        .await
        .expect("User creation failed");
    assert_eq!(response.status(), StatusCode::CREATED);
    openapi::checked(Method::POST, response).await
}

async fn patch_reader(server: &TestServer, reader_id: &str, body: Value) {
    let response = server
        .admin(Method::PATCH, &format!("/api/v2/users/{reader_id}"))
        .json(&body)
        .send()
        .await
        .expect("User patch failed");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    openapi::checked(Method::PATCH, response).await;
}

async fn find_user(server: &TestServer, user_id: &str) -> Value {
    let users = openapi::checked(
        Method::GET,
        server
            .admin(Method::GET, "/api/v2/users")
            .send()
            .await
            .expect("User list failed"),
    )
    .await;
    users
        .as_array()
        .expect("User list must be an array")
        .iter()
        .find(|user| user["id"] == user_id)
        .expect("User must be listed")
        .clone()
}

/// Logs in with Basic and returns the session cookie it creates.
async fn reader_session(server: &TestServer) -> String {
    let response = server
        .request(Method::GET, "/api/v2/users/me")
        .basic_auth(READER_EMAIL, Some(READER_PASSWORD))
        .send()
        .await
        .expect("Reader login failed");
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get(SET_COOKIE)
        .expect("Basic login must create a session cookie")
        .to_str()
        .expect("Cookie must be ASCII");
    cookie
        .split(';')
        .next()
        .expect("Cookie must have a name-value pair")
        .to_owned()
}

async fn reader_me_status(server: &TestServer, session: &str) -> StatusCode {
    server
        .request(Method::GET, "/api/v2/users/me")
        .header(COOKIE, session)
        .send()
        .await
        .expect("Session request failed")
        .status()
}

async fn reader_me(server: &TestServer, session: &str) -> Value {
    let response = server
        .request(Method::GET, "/api/v2/users/me")
        .header(COOKIE, session)
        .send()
        .await
        .expect("Session request failed");
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::GET, response).await
}
