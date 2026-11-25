//! Global and per-user client settings (Kotlin `ClientSettingsController` use cases).
//!
//! The server keeps the two scopes apart; clients merge user settings over global ones.

use std::collections::BTreeMap;

use komga_db::{ClientSettingsDao, Database};
use komga_types::{GlobalClientSetting, KomgaUser, UserId};

use super::SettingsError;

#[derive(Clone)]
pub struct ClientSettingsService {
    db: Database,
}

impl ClientSettingsService {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Global settings visible to `viewer`; anonymous viewers only see those allowing
    /// unauthorized access.
    ///
    /// # Errors
    /// Fails on database errors.
    pub async fn global_settings(
        &self,
        viewer: Option<&KomgaUser>,
    ) -> Result<BTreeMap<String, GlobalClientSetting>, SettingsError> {
        Ok(ClientSettingsDao::find_all_global(&self.db, viewer.is_none()).await?)
    }

    /// # Errors
    /// Fails on database errors.
    pub async fn user_settings(
        &self,
        user_id: &UserId,
    ) -> Result<BTreeMap<String, String>, SettingsError> {
        Ok(ClientSettingsDao::find_all_user(&self.db, user_id.as_str()).await?)
    }

    /// Saves all settings in one transaction.
    ///
    /// # Errors
    /// Fails on database errors, leaving every setting unchanged.
    pub async fn save_global(
        &self,
        settings: &BTreeMap<String, GlobalClientSetting>,
    ) -> Result<(), SettingsError> {
        let mut transaction = self.db.begin().await?;
        for (key, setting) in settings {
            ClientSettingsDao::save_global(&mut transaction, key, setting).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Saves all settings of `user_id` in one transaction.
    ///
    /// # Errors
    /// Fails on database errors, leaving every setting unchanged.
    pub async fn save_for_user(
        &self,
        user_id: &UserId,
        settings: &BTreeMap<String, String>,
    ) -> Result<(), SettingsError> {
        let mut transaction = self.db.begin().await?;
        for (key, value) in settings {
            ClientSettingsDao::save_for_user(&mut transaction, user_id.as_str(), key, value)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    /// # Errors
    /// Fails on database errors.
    pub async fn delete_global(&self, keys: &[String]) -> Result<(), SettingsError> {
        Ok(ClientSettingsDao::delete_global_by_keys(&self.db, keys).await?)
    }

    /// # Errors
    /// Fails on database errors.
    pub async fn delete_for_user(
        &self,
        user_id: &UserId,
        keys: &[String],
    ) -> Result<(), SettingsError> {
        Ok(ClientSettingsDao::delete_by_user_id_and_keys(&self.db, user_id.as_str(), keys).await?)
    }
}
