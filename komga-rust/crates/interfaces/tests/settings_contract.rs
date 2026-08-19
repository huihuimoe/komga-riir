use komga_interfaces::contracts::operational::{
    SettingMultiSourceDto, SettingsDto, ThumbnailSizeDto,
};
use serde_json::json;

#[test]
fn settings_contract_matches_kotlin_field_names_and_null_sources() {
    let settings = serde_json::to_value(SettingsDto {
        delete_empty_collections: true,
        delete_empty_read_lists: false,
        remember_me_duration_days: 30,
        thumbnail_size: ThumbnailSizeDto::Large,
        task_pool_size: 2,
        server_port: SettingMultiSourceDto::new(Some(25600), None, Some(25600)),
        server_context_path: SettingMultiSourceDto::new(None, None, Some(String::new())),
        kobo_proxy: false,
        kobo_port: None,
        kepubify_path: SettingMultiSourceDto::new(
            None,
            Some("/usr/bin/kepubify".to_string()),
            None,
        ),
    })
    .expect("settings should serialize");

    assert_eq!(
        settings,
        json!({
            "deleteEmptyCollections": true,
            "deleteEmptyReadLists": false,
            "rememberMeDurationDays": 30,
            "thumbnailSize": "LARGE",
            "taskPoolSize": 2,
            "serverPort": {
                "configurationSource": 25600,
                "databaseSource": null,
                "effectiveValue": 25600,
            },
            "serverContextPath": {
                "configurationSource": null,
                "databaseSource": null,
                "effectiveValue": "",
            },
            "koboProxy": false,
            "koboPort": null,
            "kepubifyPath": {
                "configurationSource": null,
                "databaseSource": "/usr/bin/kepubify",
                "effectiveValue": null,
            },
        })
    );
}
