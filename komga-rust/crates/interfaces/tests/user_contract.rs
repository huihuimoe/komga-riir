use komga_application::identity_access::{
    AuthUser, AuthUserAgeRestriction, AuthUserAgeRestrictionKind, AuthUserRole,
};
use komga_interfaces::contracts::identity_access::UserDto;
use serde_json::json;

fn user() -> AuthUser {
    AuthUser {
        id: "user-1".to_string(),
        email: "user@example.com".to_string(),
        password: String::new(),
        roles: vec![AuthUserRole::Admin],
        shared_all_libraries: false,
        shared_library_ids: vec!["library-1".to_string()],
        labels_allow: vec!["Team".to_string()],
        labels_exclude: vec!["Spoiler".to_string()],
        age_restriction: Some(AuthUserAgeRestriction {
            age: 16,
            restriction: AuthUserAgeRestrictionKind::AllowOnly,
        }),
    }
}

#[test]
fn user_dto_matches_kotlin_field_shape_and_age_restriction() {
    let payload = serde_json::to_value(UserDto::from_user(&user())).expect("user should serialize");

    assert_eq!(
        payload
            .as_object()
            .expect("user should be an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "ageRestriction",
            "email",
            "id",
            "labelsAllow",
            "labelsExclude",
            "roles",
            "sharedAllLibraries",
            "sharedLibrariesIds",
        ]
    );
    assert_eq!(payload["roles"], json!(["ADMIN", "USER"]));
    assert_eq!(
        payload["ageRestriction"],
        json!({ "age": 16, "restriction": "ALLOW_ONLY" })
    );
}

#[test]
fn user_dto_omits_null_age_restriction() {
    let mut user = user();
    user.age_restriction = None;

    let payload = serde_json::to_value(UserDto::from_user(&user)).expect("user should serialize");
    assert!(!payload.as_object().unwrap().contains_key("ageRestriction"));
}
