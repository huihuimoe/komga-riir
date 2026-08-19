use komga_interfaces::contracts::opds::{
    OpdsAuthenticationDto, OpdsAuthenticationLabelsDto, OpdsAuthenticationLinkDto,
    OpdsAuthenticationMethodDto,
};
use serde_json::json;

#[test]
fn opds_authentication_dto_preserves_protocol_shape() {
    let payload = serde_json::to_value(OpdsAuthenticationDto {
        authentication: vec![OpdsAuthenticationMethodDto {
            authentication_type: "http://opds-spec.org/auth/basic".to_string(),
            labels: OpdsAuthenticationLabelsDto {
                login: "Email".to_string(),
                password: "Password".to_string(),
            },
        }],
        title: "Komga".to_string(),
        id: "https://example.test/opds/v2/auth".to_string(),
        description: "Enter your email and password to authenticate.".to_string(),
        links: vec![OpdsAuthenticationLinkDto {
            rel: "help".to_string(),
            href: "https://komga.org".to_string(),
        }],
    })
    .expect("OPDS authentication document should serialize");

    assert_eq!(
        payload,
        json!({
            "authentication": [{
                "type": "http://opds-spec.org/auth/basic",
                "labels": {"login": "Email", "password": "Password"},
            }],
            "title": "Komga",
            "id": "https://example.test/opds/v2/auth",
            "description": "Enter your email and password to authenticate.",
            "links": [{"rel": "help", "href": "https://komga.org"}],
        })
    );
}
