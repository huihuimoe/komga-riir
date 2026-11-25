//! API key domain entity.

use serde::{Deserialize, Serialize};

use crate::ids::{ApiKeyId, UserId};

/// Domain entity representing a user's API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: ApiKeyId,
    pub user_id: UserId,
    pub created_date: String,
    pub last_modified_date: String,
    pub key: String,
    pub comment: String,
}
