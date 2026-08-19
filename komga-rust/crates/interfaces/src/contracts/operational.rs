use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuth2ClientDto {
    pub name: String,
    pub registration_id: String,
}

#[derive(Debug, Serialize)]
pub struct MessageDto {
    pub message: String,
}
