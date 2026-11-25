//! Claim status DTO.

use serde::{Deserialize, Serialize};

/// Claim status DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimStatusDto {
    #[serde(rename = "isClaimed")]
    pub is_claimed: bool,
}
