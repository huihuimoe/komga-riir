use komga_application::identity_access::{
    AuthUser, AuthUserAgeRestriction, user_response_role_names,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    pub id: String,
    pub email: String,
    pub roles: Vec<&'static str>,
    pub shared_all_libraries: bool,
    pub shared_libraries_ids: Vec<String>,
    pub labels_allow: Vec<String>,
    pub labels_exclude: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_restriction: Option<AgeRestrictionDto>,
}

impl UserDto {
    pub fn from_user(user: &AuthUser) -> Self {
        Self {
            id: user.id.clone(),
            email: user.email.clone(),
            roles: user_response_role_names(user),
            shared_all_libraries: user.shared_all_libraries,
            shared_libraries_ids: user.shared_library_ids.clone(),
            labels_allow: user.labels_allow.clone(),
            labels_exclude: user.labels_exclude.clone(),
            age_restriction: user
                .age_restriction
                .as_ref()
                .map(AgeRestrictionDto::from_model),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgeRestrictionDto {
    pub age: i64,
    pub restriction: String,
}

impl AgeRestrictionDto {
    fn from_model(value: &AuthUserAgeRestriction) -> Self {
        Self {
            age: value.age,
            restriction: value.restriction.persisted_name().to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimStatusDto {
    pub is_claimed: bool,
}
