use komga_interfaces::contracts::common::MessageDto;
use komga_interfaces::contracts::operational::OAuth2ClientDto;
use serde_json::json;

#[test]
fn operational_contracts_match_kotlin_field_names() {
    let provider = serde_json::to_value(OAuth2ClientDto {
        name: "GitHub".to_string(),
        registration_id: "github".to_string(),
    })
    .expect("oauth provider should serialize");
    assert_eq!(
        provider,
        json!({ "name": "GitHub", "registrationId": "github" })
    );

    let error = serde_json::to_value(MessageDto {
        message: "failed to delete tasks".to_string(),
    })
    .expect("operational message should serialize");
    assert_eq!(error, json!({ "message": "failed to delete tasks" }));
}
