//! User lookup, authentication, and lifecycle use cases.

use std::collections::BTreeSet;

use komga_db::{DbTransaction, KomgaUserDao, LibraryDao};
use komga_types::{AgeRestriction, ContentRestrictions, KomgaUser, LibraryId, UserId, UserRole};
use tracing::info;

use super::{IdentityError, IdentityService, hash_password, verify_password};

/// Libraries shared with a user; `all` overrides `library_ids`.
#[derive(Debug, Clone)]
pub struct LibrarySharing {
    pub all: bool,
    /// Requested IDs; IDs naming no existing library are dropped.
    pub library_ids: BTreeSet<LibraryId>,
}

impl Default for LibrarySharing {
    /// New users see all libraries unless told otherwise.
    fn default() -> Self {
        Self {
            all: true,
            library_ids: BTreeSet::new(),
        }
    }
}

/// Input for an administrator-created user.
#[derive(Debug, Clone)]
pub struct NewUser {
    pub email: String,
    pub password: String,
    pub roles: BTreeSet<UserRole>,
    pub library_sharing: LibrarySharing,
    pub restrictions: ContentRestrictions,
}

/// Partial user update; `None` keeps the stored value.
#[derive(Debug, Clone, Default)]
pub struct UserUpdate {
    pub roles: Option<BTreeSet<UserRole>>,
    pub library_sharing: Option<LibrarySharing>,
    /// `Some(None)` removes the age restriction.
    pub age_restriction: Option<Option<AgeRestriction>>,
    pub labels_allow: Option<BTreeSet<String>>,
    pub labels_exclude: Option<BTreeSet<String>>,
}

impl IdentityService {
    /// Check whether the server is already claimed (i.e. has at least one user).
    pub async fn is_claimed(&self) -> Result<bool, IdentityError> {
        let count = KomgaUserDao::count(&self.db).await?;
        Ok(count > 0)
    }

    /// Find a user by their unique ID.
    pub async fn find_by_id(&self, id: &str) -> Result<Option<KomgaUser>, IdentityError> {
        let user = KomgaUserDao::find_by_id(&self.db, id).await?;
        Ok(user)
    }

    /// Find a user by their email address.
    pub async fn find_by_email(&self, email: &str) -> Result<Option<KomgaUser>, IdentityError> {
        let user = KomgaUserDao::find_by_email_ignore_case(&self.db, email).await?;
        Ok(user)
    }

    /// List all registered users.
    pub async fn list_users(&self) -> Result<Vec<KomgaUser>, IdentityError> {
        let users = KomgaUserDao::find_all(&self.db).await?;
        Ok(users)
    }

    /// Authenticate a user by email and plaintext password.
    pub async fn authenticate(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Option<KomgaUser>, IdentityError> {
        let Some(user) = self.find_by_email(email).await? else {
            return Ok(None);
        };

        let valid = verify_password(password.to_owned(), user.password_hash.clone()).await?;
        if valid { Ok(Some(user)) } else { Ok(None) }
    }

    /// Create a new user (admin operation).
    pub async fn create_user(&self, new_user: NewUser) -> Result<KomgaUser, IdentityError> {
        let password_hash = hash_password(new_user.password).await?;
        // Checked inside the write transaction so that concurrent creations cannot both pass.
        let mut tx = self.db.begin_immediate().await?;
        if KomgaUserDao::exists_by_email_ignore_case(&mut tx, &new_user.email).await? {
            return Err(IdentityError::UserAlreadyExists(new_user.email));
        }
        let (shared_all_libraries, shared_library_ids) =
            resolve_library_sharing(&mut tx, new_user.library_sharing).await?;
        let user = KomgaUser {
            id: UserId::generate(),
            email: new_user.email,
            password_hash,
            roles: new_user.roles,
            shared_all_libraries,
            shared_library_ids,
            restrictions: new_user.restrictions,
        };

        KomgaUserDao::insert(&mut tx, &user).await?;
        tx.commit().await?;
        info!(email = %user.email, user_id = %user.id, "User created successfully");

        Ok(user)
    }

    /// Update user password.
    /// If `expire_sessions` is true, invalidates existing sessions.
    pub async fn update_password(
        &self,
        user_id: &str,
        new_password: &str,
        expire_sessions: bool,
    ) -> Result<(), IdentityError> {
        let Some(_user) = KomgaUserDao::find_by_id(&self.db, user_id).await? else {
            return Err(IdentityError::UserNotFound);
        };

        let new_hash = hash_password(new_password.to_owned()).await?;
        let mut tx = self.db.begin_immediate().await?;
        KomgaUserDao::update_password(&mut tx, user_id, &new_hash).await?;
        tx.commit().await?;

        if expire_sessions {
            self.expire_sessions(user_id);
            info!(user_id = %user_id, "User sessions invalidated due to password update");
        }

        Ok(())
    }

    /// Update passwords for multiple users in one atomic transaction with distinct salts.
    pub async fn update_passwords(
        &self,
        user_ids: &[UserId],
        new_password: &str,
    ) -> Result<(), IdentityError> {
        if user_ids.is_empty() {
            return Ok(());
        }
        let mut updates = Vec::with_capacity(user_ids.len());
        for user_id in user_ids {
            updates.push((user_id, hash_password(new_password.to_owned()).await?));
        }
        let mut tx = self.db.begin_immediate().await?;
        for (user_id, new_hash) in &updates {
            KomgaUserDao::update_password(&mut tx, user_id.as_str(), new_hash).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Update roles, library sharing, and content restrictions.
    ///
    /// Any change to what the user may see or do revokes their sessions.
    pub async fn update_user(
        &self,
        user_id: &str,
        update: UserUpdate,
    ) -> Result<KomgaUser, IdentityError> {
        let Some(existing) = KomgaUserDao::find_by_id(&self.db, user_id).await? else {
            return Err(IdentityError::UserNotFound);
        };

        let mut tx = self.db.begin_immediate().await?;
        let mut user = existing.clone();
        if let Some(roles) = update.roles {
            user.roles = roles;
        }
        if let Some(sharing) = update.library_sharing {
            (user.shared_all_libraries, user.shared_library_ids) =
                resolve_library_sharing(&mut tx, sharing).await?;
        }
        let current = &existing.restrictions;
        user.restrictions = ContentRestrictions::new(
            update
                .age_restriction
                .unwrap_or_else(|| current.age_restriction()),
            update
                .labels_allow
                .unwrap_or_else(|| current.labels_allow().clone()),
            update
                .labels_exclude
                .unwrap_or_else(|| current.labels_exclude().clone()),
        );

        KomgaUserDao::update(&mut tx, &user).await?;
        tx.commit().await?;

        let expire_sessions = existing.roles != user.roles
            || existing.restrictions != user.restrictions
            || existing.shared_all_libraries != user.shared_all_libraries
            || existing.shared_library_ids != user.shared_library_ids;
        if expire_sessions {
            self.expire_sessions(user_id);
        }

        Ok(user)
    }

    /// Delete user by ID and revoke existing sessions.
    pub async fn delete_user(
        &self,
        user_id: &str,
        current_admin_id: &str,
    ) -> Result<(), IdentityError> {
        if user_id == current_admin_id {
            return Err(IdentityError::CannotDeleteSelf);
        }

        let mut tx = self.db.begin_immediate().await?;
        let deleted = KomgaUserDao::delete(&mut tx, user_id).await?;
        if !deleted {
            tx.rollback().await?;
            return Err(IdentityError::UserNotFound);
        }
        tx.commit().await?;

        self.expire_sessions(user_id);
        info!(user_id = %user_id, "User deleted and sessions revoked");

        Ok(())
    }
}

async fn resolve_library_sharing(
    transaction: &mut DbTransaction,
    sharing: LibrarySharing,
) -> Result<(bool, BTreeSet<LibraryId>), IdentityError> {
    if sharing.all {
        Ok((true, BTreeSet::new()))
    } else {
        let existing = LibraryDao::find_existing_ids(transaction, &sharing.library_ids).await?;
        Ok((false, existing))
    }
}
