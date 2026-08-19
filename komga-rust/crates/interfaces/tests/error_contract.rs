use komga_interfaces::contracts::common::{ErrorMessageDto, ValidationErrorDto, ViolationDto};
use serde_json::json;

#[test]
fn common_error_contracts_match_kotlin_field_names() {
    assert_eq!(
        serde_json::to_value(ErrorMessageDto {
            error: "Bad Request".to_string(),
        })
        .expect("error message should serialize"),
        json!({ "error": "Bad Request" })
    );

    assert_eq!(
        serde_json::to_value(ValidationErrorDto {
            violations: vec![ViolationDto {
                field_name: Some("name".to_string()),
                message: Some("must not be blank".to_string()),
            }],
        })
        .expect("validation error should serialize"),
        json!({
            "violations": [{
                "fieldName": "name",
                "message": "must not be blank"
            }]
        })
    );
}
