//! Initial server claim use case.

use komga_db::KomgaUserDao;
use std::collections::BTreeSet;

use komga_types::{ContentRestrictions, KomgaUser, UserId, UserRole};
use tracing::info;

use super::{IdentityError, IdentityService, hash_password};

impl IdentityService {
    /// Attempt to claim the server by creating the initial admin user.
    pub async fn claim_server(
        &self,
        email: &str,
        password: &str,
    ) -> Result<KomgaUser, IdentityError> {
        // Fast-path check before paying bcrypt cost; the write transaction re-checks atomically.
        if KomgaUserDao::count(&self.db).await? > 0 {
            return Err(IdentityError::AlreadyClaimed);
        }

        let password_hash = hash_password(password.to_owned()).await?;

        // Concurrent claims must serialize so the loser sees the claimed state.
        let mut tx = self.db.begin_immediate().await?;
        let count = KomgaUserDao::count_in_transaction(&mut tx).await?;

        if count > 0 {
            tx.rollback().await?;
            return Err(IdentityError::AlreadyClaimed);
        }

        let user = KomgaUser {
            id: UserId::generate(),
            email: email.to_string(),
            password_hash,
            roles: UserRole::ALL.into(),
            shared_all_libraries: true,
            shared_library_ids: BTreeSet::new(),
            restrictions: ContentRestrictions::default(),
        };

        KomgaUserDao::insert(&mut tx, &user).await?;
        tx.commit().await?;
        info!(email = %user.email, user_id = %user.id, "Server successfully claimed by initial admin");

        Ok(user)
    }
}
