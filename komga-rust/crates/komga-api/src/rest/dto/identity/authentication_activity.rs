//! Kotlin `AuthenticationActivityDto`.

use komga_types::AuthenticationActivity;
use serde::Serialize;

use crate::rest::dto::JsonDateTime;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticationActivityDto {
    user_id: Option<String>,
    email: Option<String>,
    api_key_id: Option<String>,
    api_key_comment: Option<String>,
    ip: Option<String>,
    user_agent: Option<String>,
    success: bool,
    error: Option<String>,
    date_time: JsonDateTime,
    source: Option<String>,
}

impl From<AuthenticationActivity> for AuthenticationActivityDto {
    fn from(activity: AuthenticationActivity) -> Self {
        Self {
            user_id: activity.user_id,
            email: activity.email,
            api_key_id: activity.api_key_id,
            api_key_comment: activity.api_key_comment,
            ip: activity.ip,
            user_agent: activity.user_agent,
            success: activity.success,
            error: activity.error,
            date_time: JsonDateTime(activity.date_time),
            source: activity.source,
        }
    }
}
