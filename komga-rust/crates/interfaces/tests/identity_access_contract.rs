use komga_application::identity_access::{PersistedApiKey, PersistedAuthenticationActivity};
use komga_interfaces::contracts::identity_access::{ApiKeyDto, AuthenticationActivityDto};
use serde_json::json;

#[test]
fn api_key_and_authentication_activity_contracts_match_kotlin_shape() {
    let api_key = ApiKeyDto::from_persisted(
        &PersistedApiKey {
            id: "key-1".to_string(),
            user_id: "user-1".to_string(),
            key: "secret".to_string(),
            comment: "reader".to_string(),
            created_date: Some("2024-01-01 00:00:00".to_string()),
            last_modified_date: Some("2024-01-02 00:00:00".to_string()),
        },
        true,
    )
    .expect("api key should map");
    assert_eq!(
        serde_json::to_value(api_key).expect("api key should serialize"),
        json!({
            "id": "key-1",
            "userId": "user-1",
            "key": "******",
            "comment": "reader",
            "createdDate": "2024-01-01T00:00:00Z",
            "lastModifiedDate": "2024-01-02T00:00:00Z"
        })
    );

    let activity = AuthenticationActivityDto::from_persisted(&PersistedAuthenticationActivity {
        user_id: Some("user-1".to_string()),
        email: Some("reader@example.com".to_string()),
        ip: None,
        user_agent: Some("device".to_string()),
        success: true,
        error: None,
        date_time: "2024-01-03 00:00:00".to_string(),
        source: Some("ApiKey".to_string()),
        api_key_id: Some("key-1".to_string()),
        api_key_comment: Some("reader".to_string()),
    })
    .expect("authentication activity should map");
    assert_eq!(
        serde_json::to_value(activity).expect("activity should serialize"),
        json!({
            "userId": "user-1",
            "email": "reader@example.com",
            "apiKeyId": "key-1",
            "apiKeyComment": "reader",
            "ip": null,
            "userAgent": "device",
            "success": true,
            "error": null,
            "dateTime": "2024-01-03T00:00:00Z",
            "source": "ApiKey"
        })
    );
}
