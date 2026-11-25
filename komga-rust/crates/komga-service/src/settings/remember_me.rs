//! Remember-me signing settings, read once at startup like Spring's `TokenBasedRememberMeServices`.

use std::sync::Arc;

use komga_db::{Database, ServerSettingsDao};

use super::SettingsError;
use super::server::{REMEMBER_ME_KEY, ServerSettings, generate_remember_me_key};

const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

/// Immutable remember-me configuration for the process lifetime.
#[derive(Clone)]
pub struct RememberMeSettings {
    key: Arc<str>,
    validity_seconds: i32,
}

impl RememberMeSettings {
    /// Load the configured key, persisting a generated one if missing, and the duration from
    /// `settings`.
    pub async fn load(
        database: &Database,
        settings: &ServerSettings,
    ) -> Result<Self, SettingsError> {
        let key = match ServerSettingsDao::get(database, REMEMBER_ME_KEY).await? {
            Some(key) => key,
            None => {
                ServerSettingsDao::get_or_insert(
                    database,
                    REMEMBER_ME_KEY,
                    &generate_remember_me_key()?,
                )
                .await?
            }
        };
        if key.is_empty() {
            return Err(SettingsError::InvalidStoredValue {
                key: REMEMBER_ME_KEY,
                value: key,
            });
        }

        // Kotlin `Int.days.inWholeSeconds.toInt()` wraps instead of failing.
        let validity_seconds =
            (i64::from(settings.remember_me_duration_days) * SECONDS_PER_DAY) as i32;

        Ok(Self {
            key: Arc::from(key),
            validity_seconds,
        })
    }

    /// Return the signing key without copying it.
    #[must_use]
    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    /// Return the configured validity in seconds, matching Spring's integer setting.
    #[must_use]
    pub const fn validity_seconds(&self) -> i32 {
        self.validity_seconds
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
    use komga_types::{ContentRestrictions, KomgaUser, UserId};

    use crate::SessionRegistry;

    use super::RememberMeSettings;

    #[test]
    fn validates_spring_security_6_5_remember_me_token() {
        let settings = RememberMeSettings {
            key: "known-secret".into(),
            validity_seconds: 31_536_000,
        };
        let user = KomgaUser {
            id: UserId::from("user-1"),
            email: "remember@example.com".to_owned(),
            password_hash: "known-password-hash".to_owned(),
            roles: BTreeSet::new(),
            shared_all_libraries: true,
            shared_library_ids: BTreeSet::new(),
            restrictions: ContentRestrictions::default(),
        };
        let token = "cmVtZW1iZXIlNDBleGFtcGxlLmNvbTo0MTAyNDQ0ODAwMDAwOlNIQTI1NjphZDRiNDViZTBhYjk3OTFmOWQzMDMwNzdlNTQ1OGRkMGNlNTlhOWZlMWM2N2NmNTExMmRjMWM1NzU2MDc3MWQ1";
        let sessions = SessionRegistry::new();

        assert!(sessions.validate_remember_me_token(token, &user, &settings));
        let padded_token = STANDARD.encode(STANDARD_NO_PAD.decode(token).unwrap());
        assert!(sessions.validate_remember_me_token(&padded_token, &user, &settings));
    }

    #[test]
    fn negative_validity_uses_spring_default_token_expiry() {
        let settings = RememberMeSettings {
            key: "known-secret".into(),
            validity_seconds: -1,
        };
        let user = KomgaUser {
            id: UserId::from("user-1"),
            email: "remember@example.com".to_owned(),
            password_hash: "known-password-hash".to_owned(),
            roles: BTreeSet::new(),
            shared_all_libraries: true,
            shared_library_ids: BTreeSet::new(),
            restrictions: ContentRestrictions::default(),
        };

        let now_millis = || {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
        };
        let issued_after = now_millis();
        let token = SessionRegistry::new().issue_remember_me_token(&user, &settings);
        let issued_before = now_millis();
        let decoded = STANDARD_NO_PAD.decode(token).unwrap();
        let decoded = String::from_utf8(decoded).unwrap();
        let expiry_millis = decoded.split(':').nth(1).unwrap().parse::<u64>().unwrap();
        let fourteen_days_millis = 14 * 24 * 60 * 60 * 1000;

        assert!(expiry_millis >= issued_after + fourteen_days_millis);
        assert!(expiry_millis <= issued_before + fourteen_days_millis);
    }
}
