use komga_types::UserId;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct SessionExpiredSseDto<'a> {
    pub user_id: &'a UserId,
}
