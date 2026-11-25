//! Server settings DTOs (`SettingsDto`, `SettingsUpdateDto`).

use komga_service::ServerSettingsUpdate;
use komga_types::ThumbnailSize;
use serde::{Deserialize, Serialize};

use crate::error::Violation;
use crate::rest::dto::present;
use crate::rest::validation;

/// Absent fields are omitted (`@JsonInclude(NON_NULL)`); non-admins only see the upload limit.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_empty_collections: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_empty_read_lists: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remember_me_duration_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_size: Option<ThumbnailSizeDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_pool_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_port: Option<SettingMultiSource<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_context_path: Option<SettingMultiSource<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kobo_proxy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kobo_port: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kepubify_path: Option<SettingMultiSource<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_upload_file_size_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_ui_as_default: Option<bool>,
}

/// Unlike its parent, this class has no `NON_NULL` rule, so absent sources serialize as `null`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingMultiSource<T> {
    pub configuration_source: Option<T>,
    pub database_source: Option<T>,
    pub effective_value: Option<T>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ThumbnailSizeDto {
    Default,
    Medium,
    Large,
    XLarge,
}

impl From<ThumbnailSize> for ThumbnailSizeDto {
    fn from(size: ThumbnailSize) -> Self {
        match size {
            ThumbnailSize::Default => Self::Default,
            ThumbnailSize::Medium => Self::Medium,
            ThumbnailSize::Large => Self::Large,
            ThumbnailSize::XLarge => Self::XLarge,
        }
    }
}

impl From<ThumbnailSizeDto> for ThumbnailSize {
    fn from(size: ThumbnailSizeDto) -> Self {
        match size {
            ThumbnailSizeDto::Default => Self::Default,
            ThumbnailSizeDto::Medium => Self::Medium,
            ThumbnailSizeDto::Large => Self::Large,
            ThumbnailSizeDto::XLarge => Self::XLarge,
        }
    }
}

/// Server settings PATCH body. Explicit `null` deletes the nullable settings (server port,
/// context path, Kobo port, kepubify path) and leaves the others unchanged.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsUpdateDto {
    pub delete_empty_collections: Option<bool>,
    pub delete_empty_read_lists: Option<bool>,
    pub remember_me_duration_days: Option<i64>,
    pub renew_remember_me_key: Option<bool>,
    pub thumbnail_size: Option<ThumbnailSizeDto>,
    pub task_pool_size: Option<i32>,
    #[serde(default, deserialize_with = "present")]
    pub server_port: Option<Option<i32>>,
    #[serde(default, deserialize_with = "present")]
    pub server_context_path: Option<Option<String>>,
    pub kobo_proxy: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    pub kobo_port: Option<Option<i32>>,
    #[serde(default, deserialize_with = "present")]
    pub kepubify_path: Option<Option<String>>,
    pub next_ui_as_default: Option<bool>,
}

impl SettingsUpdateDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        if let Some(days) = self.remember_me_duration_days {
            validation::positive("rememberMeDurationDays", days, &mut violations);
        }
        if let Some(size) = self.task_pool_size {
            validation::positive("taskPoolSize", size.into(), &mut violations);
        }
        if let Some(Some(port)) = self.server_port {
            validation::port("serverPort", port, &mut violations);
        }
        if let Some(Some(path)) = &self.server_context_path {
            validation::context_path("serverContextPath", path, &mut violations);
        }
        if let Some(Some(port)) = self.kobo_port {
            validation::port("koboPort", port, &mut violations);
        }
        violations
    }

    pub(crate) fn into_update(self) -> ServerSettingsUpdate {
        ServerSettingsUpdate {
            delete_empty_collections: self.delete_empty_collections,
            delete_empty_read_lists: self.delete_empty_read_lists,
            remember_me_duration_days: self.remember_me_duration_days,
            renew_remember_me_key: self.renew_remember_me_key == Some(true),
            thumbnail_size: self.thumbnail_size.map(ThumbnailSize::from),
            task_pool_size: self.task_pool_size,
            server_port: self.server_port,
            server_context_path: self.server_context_path,
            kobo_proxy: self.kobo_proxy,
            kobo_port: self.kobo_port,
            kepubify_path: self.kepubify_path,
            next_ui_as_default: self.next_ui_as_default,
        }
    }
}
