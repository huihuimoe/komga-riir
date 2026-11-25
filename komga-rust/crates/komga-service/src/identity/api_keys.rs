//! API key creation, authentication, and revocation use cases.

use komga_db::KomgaUserDao;
use komga_types::{ApiKey, ApiKeyId, KomgaUser};
use sha2::{Digest, Sha512};
use tracing::info;

use super::{IdentityError, IdentityService};

impl IdentityService {
    /// Authenticate a user by raw plaintext API key (SHA-512 hashed for lookup).
    pub async fn authenticate_api_key(
        &self,
        raw_api_key: &str,
    ) -> Result<Option<(KomgaUser, ApiKey)>, IdentityError> {
        let key_hash = Self::hash_api_key(raw_api_key);
        let result = KomgaUserDao::find_user_by_api_key_hash(&self.db, &key_hash).await?;
        Ok(result)
    }

    /// Create and persist a new API key for the user. Returns the plaintext key.
    pub async fn create_api_key(
        &self,
        user_id: &str,
        comment: &str,
    ) -> Result<ApiKey, IdentityError> {
        let trimmed_comment = comment.trim();
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).expect("Failed to generate random bytes");
        let plain_key = hex::encode(bytes);
        let key_hash = Self::hash_api_key(&plain_key);
        let key_id = ApiKeyId::generate();

        // Duplicate-comment check and insert share an immediate write transaction to avoid TOCTOU.
        let mut tx = self.db.begin_immediate().await?;
        if KomgaUserDao::exists_api_key_by_comment_and_user_id(&mut tx, trimmed_comment, user_id)
            .await?
        {
            tx.rollback().await?;
            return Err(IdentityError::ApiKeyCommentDuplicate);
        }

        let mut api_key = KomgaUserDao::insert_api_key(
            &mut tx,
            key_id.as_str(),
            user_id,
            &key_hash,
            trimmed_comment,
        )
        .await?;
        tx.commit().await?;

        info!(user_id = %user_id, comment = %trimmed_comment, "API key created successfully");

        api_key.key = plain_key;
        Ok(api_key)
    }

    /// List API keys for a user.
    pub async fn list_api_keys(&self, user_id: &str) -> Result<Vec<ApiKey>, IdentityError> {
        let keys = KomgaUserDao::find_api_keys_by_user_id(&self.db, user_id).await?;
        Ok(keys)
    }

    /// Delete/revoke an API key by ID.
    pub async fn delete_api_key(&self, key_id: &str, user_id: &str) -> Result<(), IdentityError> {
        let mut tx = self.db.begin_immediate().await?;
        let deleted = KomgaUserDao::delete_api_key_by_id(&mut tx, key_id, user_id).await?;
        if !deleted {
            tx.rollback().await?;
            return Err(IdentityError::ApiKeyNotFound);
        }
        tx.commit().await?;
        info!(key_id = %key_id, user_id = %user_id, "API key revoked successfully");
        Ok(())
    }

    /// Hash API key with SHA-512 matching baseline TokenEncoder (Sha512DigestUtils.shaHex).
    #[must_use]
    pub fn hash_api_key(key: &str) -> String {
        let mut hasher = Sha512::new();
        hasher.update(key.as_bytes());
        hex::encode(hasher.finalize())
    }
}
