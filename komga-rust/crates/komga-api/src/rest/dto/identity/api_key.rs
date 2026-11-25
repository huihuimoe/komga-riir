//! API key request and response DTOs.

use komga_types::{ApiKey, ApiKeyId, UserId};
use serde::{Deserialize, Serialize};

use crate::error::Violation;
use crate::rest::validation;

/// API key creation request DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyRequestDto {
    pub comment: String,
}

impl ApiKeyRequestDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::not_blank("comment", &self.comment, &mut violations);
        violations
    }
}

/// API key response DTO matching baseline ApiKeyDto.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyDto {
    pub id: ApiKeyId,
    pub key: String,
    pub comment: String,
    #[serde(rename = "userId")]
    pub user_id: UserId,
    #[serde(rename = "createdDate")]
    pub created_date: String,
    #[serde(rename = "lastModifiedDate")]
    pub last_modified_date: String,
}

impl From<ApiKey> for ApiKeyDto {
    fn from(key: ApiKey) -> Self {
        Self {
            id: key.id,
            key: key.key,
            comment: key.comment,
            user_id: key.user_id,
            created_date: format_sqlite_utc_timestamp(&key.created_date),
            last_modified_date: format_sqlite_utc_timestamp(&key.last_modified_date),
        }
    }
}

fn format_sqlite_utc_timestamp(timestamp: &str) -> String {
    timestamp.split_once(' ').map_or_else(
        || timestamp.to_owned(),
        |(date, time)| format!("{date}T{time}Z"),
    )
}

impl ApiKeyDto {
    pub(crate) fn redacted(mut self) -> Self {
        self.key = "******".to_owned();
        self
    }
}
