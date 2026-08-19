use komga_application::operational::{ClientGlobalSetting, ClientUserSetting, ClientUserSettings};
use komga_interfaces::contracts::client_settings::{
    client_settings_global_dto, client_settings_user_dto,
};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn client_settings_contract_preserves_dynamic_keys_and_kotlin_omission() {
    let global = client_settings_global_dto(&BTreeMap::from([(
        "appearance.mode".to_string(),
        ClientGlobalSetting {
            value: "dark".to_string(),
            allow_unauthorized: false,
        },
    )]));
    assert_eq!(
        serde_json::to_value(global).expect("global settings should serialize"),
        json!({
            "appearance.mode": {
                "value": "dark",
                "allowUnauthorized": false
            }
        })
    );

    let user: ClientUserSettings = BTreeMap::from([(
        "appearance.mode".to_string(),
        ClientUserSetting {
            value: "light".to_string(),
        },
    )]);
    assert_eq!(
        serde_json::to_value(client_settings_user_dto(&user))
            .expect("user settings should serialize"),
        json!({ "appearance.mode": { "value": "light" } })
    );
}
