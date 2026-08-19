use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct OpdsAuthenticationDto {
    pub authentication: Vec<OpdsAuthenticationMethodDto>,
    pub title: String,
    pub id: String,
    pub description: String,
    pub links: Vec<OpdsAuthenticationLinkDto>,
}

#[derive(Debug, Serialize)]
pub struct OpdsAuthenticationMethodDto {
    #[serde(rename = "type")]
    pub authentication_type: String,
    pub labels: OpdsAuthenticationLabelsDto,
}

#[derive(Debug, Serialize)]
pub struct OpdsAuthenticationLabelsDto {
    pub login: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct OpdsAuthenticationLinkDto {
    pub rel: String,
    pub href: String,
}
