use komga_interfaces::contracts::common::SpringErrorDto;
use komga_interfaces::contracts::identity_access::ClaimStatusDto;
use serde_json::json;

#[test]
fn claim_contracts_match_kotlin_and_spring_field_names() {
    let status = serde_json::to_value(ClaimStatusDto { is_claimed: true })
        .expect("claim status should serialize");
    assert_eq!(status, json!({ "isClaimed": true }));

    let error = serde_json::to_value(SpringErrorDto {
        error: "Bad Request".to_string(),
        message: "This server has already been claimed".to_string(),
        path: "/api/v1/claim".to_string(),
        status: 400,
        timestamp: 1_700_000_000_000,
    })
    .expect("spring error should serialize");
    assert_eq!(
        error,
        json!({
            "error": "Bad Request",
            "message": "This server has already been claimed",
            "path": "/api/v1/claim",
            "status": 400,
            "timestamp": 1_700_000_000_000_u64
        })
    );
}
