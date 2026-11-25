use komga_service::IdentityService;
use reqwest::header::USER_AGENT;
use reqwest::{Method, StatusCode};

use super::{assert_default_error, assert_tsid_id, create_user, persisted_identity};
use crate::support::{ADMIN_EMAIL, TestServer, json};

const API_KEYS: &str = "/api/v2/users/me/api-keys";

struct CreatedApiKey {
    id: String,
    key: String,
}

#[tokio::test]
async fn test_api_key_lifecycle_and_authentication() {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create tempdir");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let identity = persisted_identity(temp_dir.path()).await;
    let admin_id = identity
        .find_by_email(ADMIN_EMAIL)
        .await
        .expect("Failed to read admin")
        .expect("Admin must exist")
        .id
        .to_string();

    let created = create_api_key(&server, &admin_id).await;
    test_api_key_authentication(&server, &identity, &admin_id, &created).await;
    test_api_key_storage_and_listing(&server, &identity, &admin_id, &created).await;
    test_api_key_owner_scope(&server, &created).await;
    test_api_key_revocation(&server, &identity, &created).await;
    server.stop().await;
}

fn me_with_key(server: &TestServer, key: &str) -> reqwest::RequestBuilder {
    server
        .request(Method::GET, "/api/v2/users/me")
        .header("X-API-Key", key)
}

async fn create_api_key(server: &TestServer, user_id: &str) -> CreatedApiKey {
    let create = || {
        server
            .admin(Method::POST, API_KEYS)
            .json(&serde_json::json!({ "comment": "tachiyomi sync" }))
            .send()
    };
    let response = create().await.expect("API-key creation request failed");
    assert_eq!(response.status(), StatusCode::OK);
    let key_dto = json(response).await;
    let key = key_dto["key"]
        .as_str()
        .expect("API-key create response must contain the plaintext key")
        .to_owned();
    let id = key_dto["id"]
        .as_str()
        .expect("API-key create response must contain the key ID")
        .to_owned();
    assert_tsid_id(&key_dto["id"]);
    assert_eq!(key_dto["userId"], user_id);
    assert_eq!(key.len(), 32);
    assert!(key.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(key_dto["comment"], "tachiyomi sync");
    for field in ["createdDate", "lastModifiedDate"] {
        let timestamp = key_dto[field]
            .as_str()
            .expect("API key response must include its audit timestamp");
        assert!(
            timestamp.contains('T') && timestamp.ends_with('Z'),
            "{timestamp}"
        );
    }

    let duplicate = create().await.expect("Duplicate API-key request failed");
    assert_eq!(duplicate.status(), StatusCode::BAD_REQUEST);
    assert_default_error(&json(duplicate).await, 400, "ERR_1034", API_KEYS);

    CreatedApiKey { id, key }
}

async fn test_api_key_authentication(
    server: &TestServer,
    identity: &IdentityService,
    user_id: &str,
    created: &CreatedApiKey,
) {
    let response = me_with_key(server, &created.key)
        .header(USER_AGENT, "key-client/1.0")
        .header("X-Real-IP", "198.51.100.7")
        .send()
        .await
        .expect("API-key authentication request failed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["email"], ADMIN_EMAIL);

    let activity = super::activities(identity)
        .await
        .into_iter()
        .find(|activity| {
            activity.source.as_deref() == Some("ApiKey")
                && activity.api_key_id.as_deref() == Some(created.id.as_str())
                && activity.ip.as_deref() == Some("198.51.100.7")
        })
        .expect("API key authentication must be recorded");
    assert_eq!(activity.user_id.as_deref(), Some(user_id));
    assert_eq!(activity.email.as_deref(), Some(ADMIN_EMAIL));
    assert_eq!(activity.user_agent.as_deref(), Some("key-client/1.0"));
    assert!(activity.success);
    assert_eq!(activity.api_key_comment.as_deref(), Some("tachiyomi sync"));
}

async fn test_api_key_storage_and_listing(
    server: &TestServer,
    identity: &IdentityService,
    user_id: &str,
    created: &CreatedApiKey,
) {
    let persisted = identity
        .list_api_keys(user_id)
        .await
        .expect("Failed to list persisted API keys")
        .into_iter()
        .find(|key| key.id.as_str() == created.id)
        .expect("Created API key must be persisted");
    assert_eq!(persisted.key, IdentityService::hash_api_key(&created.key));

    let response = server
        .request(Method::GET, API_KEYS)
        .header("X-API-Key", &created.key)
        .send()
        .await
        .expect("API-key listing request failed");
    assert_eq!(response.status(), StatusCode::OK);
    let keys = json(response).await;
    let keys = keys
        .as_array()
        .expect("API-key listing response must be an array");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0]["key"], "******");
    assert_eq!(keys[0]["userId"], user_id);
    assert!(keys[0]["createdDate"].is_string());
    assert!(keys[0]["lastModifiedDate"].is_string());
}

async fn test_api_key_owner_scope(server: &TestServer, created: &CreatedApiKey) {
    create_user(server, "api-key-reader@example.com", "readerpass").await;
    let response = server
        .request(Method::DELETE, &format!("{API_KEYS}/{}", created.id))
        .basic_auth("api-key-reader@example.com", Some("readerpass"))
        .send()
        .await
        .expect("Cross-user API-key deletion request failed");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let retry = me_with_key(server, &created.key)
        .send()
        .await
        .expect("Request failed");
    assert_eq!(retry.status(), StatusCode::OK);
}

async fn test_api_key_revocation(
    server: &TestServer,
    identity: &IdentityService,
    created: &CreatedApiKey,
) {
    let response = server
        .admin(Method::DELETE, &format!("{API_KEYS}/{}", created.id))
        .send()
        .await
        .expect("API-key revocation request failed");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let revoked = me_with_key(server, &created.key)
        .header(USER_AGENT, "revoked-key-client/1.0")
        .header("X-Real-IP", "198.51.100.8")
        .send()
        .await
        .expect("Revoked API-key request failed");
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

    let activities = super::activities(identity).await;
    assert!(activities.iter().any(|activity| {
        !activity.success
            && activity.source.as_deref() == Some("ApiKey")
            && activity.error.as_deref() == Some("Bad credentials")
            && activity.ip.as_deref() == Some("198.51.100.8")
            && activity.user_agent.as_deref() == Some("revoked-key-client/1.0")
    }));
}
