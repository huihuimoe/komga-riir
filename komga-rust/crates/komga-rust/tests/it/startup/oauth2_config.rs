use komga_rust::config::ConfigError;

use super::config::{args_for, config_dir_with, resolve};

#[test]
fn oauth2_providers_bind_from_files_then_the_environment() {
    let yaml = r#"
spring:
  security:
    oauth2:
      client:
        registration:
          my_idp:
            provider: keycloak
            client-id: komga
            client-secret: from-file
            scope: [openid, email]
          GitHub:
            clientId: gh
            scope: read:user, user:email
        provider:
          keycloak:
            issuer-uri: https://sso.test/realms/komga
"#;
    let dir = config_dir_with(&[("application.yml", yaml)]);
    let config = resolve(
        &args_for(dir.path()),
        &[
            (
                "SPRING_SECURITY_OAUTH2_CLIENT_REGISTRATION_MY_IDP_CLIENT_SECRET",
                "from-env",
            ),
            (
                "SPRING_SECURITY_OAUTH2_CLIENT_REGISTRATION_MY_IDP_SCOPE_0",
                "openid",
            ),
            (
                "SPRING_SECURITY_OAUTH2_CLIENT_PROVIDER_KEYCLOAK_USERNAMEATTRIBUTE",
                "email",
            ),
        ],
    )
    .expect("OAuth2 properties must bind");

    let client = config.oauth2.client;
    let idp = &client.registration["my_idp"];
    assert_eq!(idp.provider.as_deref(), Some("keycloak"));
    assert_eq!(idp.client_id.as_deref(), Some("komga"));
    assert_eq!(
        idp.client_secret.as_ref().map(|s| s.expose()),
        Some("from-env")
    );
    assert_eq!(idp.scope.as_deref(), Some(&["openid".to_owned()][..]));
    let github = &client.registration["GitHub"];
    assert_eq!(github.client_id.as_deref(), Some("gh"));
    assert_eq!(
        github.scope.as_deref(),
        Some(&["read:user".to_owned(), "user:email".to_owned()][..])
    );
    let keycloak = &client.provider["keycloak"];
    assert_eq!(
        keycloak.issuer_uri.as_deref(),
        Some("https://sso.test/realms/komga")
    );
    assert_eq!(keycloak.user_name_attribute.as_deref(), Some("email"));
}

#[test]
fn unknown_oauth2_properties_are_rejected() {
    let dir = config_dir_with(&[(
        "application.properties",
        "spring.security.oauth2.client.registration.idp.client-foo=x\n",
    )]);
    let error = resolve(&args_for(dir.path()), &[]).expect_err("Unknown OAuth2 property");
    assert!(
        matches!(error, ConfigError::Unsupported { ref key, .. } if key == "spring.security.oauth2.client.registration.idp.client-foo"),
        "{error}"
    );
}
