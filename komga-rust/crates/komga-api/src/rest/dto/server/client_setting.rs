//! Client settings DTOs (`ClientSettingDto`, `ClientSettingGlobalUpdateDto`,
//! `ClientSettingUserUpdateDto`).
//!
//! The baseline validates request maps and key sets through method validation, so violation
//! field names are Hibernate Validator property paths such as
//! `saveGlobalSetting.newSettings<K>[Key].<map key>`.

use std::collections::{BTreeMap, BTreeSet};

use komga_types::GlobalClientSetting;
use serde::{Deserialize, Serialize};

use crate::error::Violation;
use crate::rest::validation;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSettingDto {
    pub value: String,
    /// Absent for user settings (`@JsonInclude(NON_NULL)`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_unauthorized: Option<bool>,
}

impl From<GlobalClientSetting> for ClientSettingDto {
    fn from(setting: GlobalClientSetting) -> Self {
        Self {
            value: setting.value,
            allow_unauthorized: Some(setting.allow_unauthorized),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSettingGlobalUpdateDto {
    pub value: String,
    pub allow_unauthorized: bool,
}

impl From<ClientSettingGlobalUpdateDto> for GlobalClientSetting {
    fn from(setting: ClientSettingGlobalUpdateDto) -> Self {
        Self {
            value: setting.value,
            allow_unauthorized: setting.allow_unauthorized,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClientSettingUserUpdateDto {
    pub value: String,
}

/// `PATCH` body: settings by key; `null` entries are rejected by validation.
pub type ClientSettingsUpdate<T> = BTreeMap<String, Option<T>>;

/// `DELETE` body: keys to delete; `null` elements pass validation and match nothing.
pub type ClientSettingKeys = Vec<Option<String>>;

/// Validates a `PATCH` body for the controller method `method` and returns the settings.
pub(crate) fn validate_update<T>(
    method: &str,
    settings: ClientSettingsUpdate<T>,
    value: impl Fn(&T) -> &str,
) -> Result<BTreeMap<String, T>, Vec<Violation>> {
    let mut violations = Vec::new();
    for (key, setting) in &settings {
        validation::client_setting_key(
            format!("{method}.newSettings<K>[{key}].<map key>"),
            key,
            &mut violations,
        );
        validation::not_null(
            format!("{method}.newSettings[{key}].<map value>"),
            setting.as_ref(),
            &mut violations,
        );
        if let Some(setting) = setting {
            validation::not_blank(
                format!("{method}.newSettings[{key}].value"),
                value(setting),
                &mut violations,
            );
        }
    }
    if !violations.is_empty() {
        return Err(violations);
    }
    Ok(settings
        .into_iter()
        .filter_map(|(key, setting)| Some((key, setting?)))
        .collect())
}

/// Validates a `DELETE` body for the controller method `method` and returns the distinct
/// non-null keys, as the baseline binds a `Set<String>`.
pub(crate) fn validate_keys(
    method: &str,
    keys: ClientSettingKeys,
) -> Result<Vec<String>, Vec<Violation>> {
    let keys: BTreeSet<String> = keys.into_iter().flatten().collect();
    let mut violations = Vec::new();
    for key in &keys {
        validation::client_setting_key(
            format!("{method}.keysToDelete[].<iterable element>"),
            key,
            &mut violations,
        );
    }
    if violations.is_empty() {
        Ok(keys.into_iter().collect())
    } else {
        Err(violations)
    }
}
