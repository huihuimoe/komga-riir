//! Server settings persisted in `SERVER_SETTINGS` (Kotlin `KomgaSettingsProvider`).
//!
//! The database is the only source; every read reflects the last committed update. Remember-me
//! settings, the server port, and the context path are read at startup and need a restart.

use std::collections::HashMap;

use komga_db::{Database, ServerSettingsDao};
use komga_scheduler::TaskPoolSize;
use komga_types::ThumbnailSize;

use super::SettingsError;

const DELETE_EMPTY_COLLECTIONS: &str = "DELETE_EMPTY_COLLECTIONS";
const DELETE_EMPTY_READLISTS: &str = "DELETE_EMPTY_READLISTS";
pub(super) const REMEMBER_ME_KEY: &str = "REMEMBER_ME_KEY";
const REMEMBER_ME_DURATION: &str = "REMEMBER_ME_DURATION";
const THUMBNAIL_SIZE: &str = "THUMBNAIL_SIZE";
const TASK_POOL_SIZE: &str = "TASK_POOL_SIZE";
const SERVER_PORT: &str = "SERVER_PORT";
const SERVER_CONTEXT_PATH: &str = "SERVER_CONTEXT_PATH";
const KOBO_PROXY: &str = "KOBO_PROXY";
const KOBO_PORT: &str = "KOBO_PORT";
const KEPUBIFY_PATH: &str = "KEPUBIFY_PATH";
const NEXTUI_AS_DEFAULT: &str = "NEXTUI_AS_DEFAULT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSettings {
    pub delete_empty_collections: bool,
    pub delete_empty_read_lists: bool,
    pub remember_me_duration_days: i32,
    pub thumbnail_size: ThumbnailSize,
    pub task_pool_size: i32,
    pub server_port: Option<i32>,
    pub server_context_path: Option<String>,
    pub kobo_proxy: bool,
    pub kobo_port: Option<i32>,
    /// Blank stored values read as unset.
    pub kepubify_path: Option<String>,
    pub next_ui_as_default: bool,
}

/// A partial update; `None` leaves a setting unchanged. For nullable settings, `Some(None)`
/// deletes the stored value.
#[derive(Debug, Clone, Default)]
pub struct ServerSettingsUpdate {
    pub delete_empty_collections: Option<bool>,
    pub delete_empty_read_lists: Option<bool>,
    /// Stored as Kotlin's `Long.toInt()`, which wraps out-of-range values.
    pub remember_me_duration_days: Option<i64>,
    pub renew_remember_me_key: bool,
    pub thumbnail_size: Option<ThumbnailSize>,
    pub task_pool_size: Option<i32>,
    pub server_port: Option<Option<i32>>,
    pub server_context_path: Option<Option<String>>,
    pub kobo_proxy: Option<bool>,
    pub kobo_port: Option<Option<i32>>,
    pub kepubify_path: Option<Option<String>>,
    pub next_ui_as_default: Option<bool>,
}

impl ServerSettings {
    /// Reads the stored settings, applying the baseline defaults to absent keys.
    ///
    /// # Errors
    /// Fails on database errors and on an unknown stored thumbnail size.
    pub async fn load(db: &Database) -> Result<Self, SettingsError> {
        let stored = Stored(ServerSettingsDao::find_all(db).await?);
        Ok(Self {
            delete_empty_collections: stored.boolean(DELETE_EMPTY_COLLECTIONS).unwrap_or(false),
            delete_empty_read_lists: stored.boolean(DELETE_EMPTY_READLISTS).unwrap_or(false),
            remember_me_duration_days: stored.int(REMEMBER_ME_DURATION).unwrap_or(365),
            thumbnail_size: stored.thumbnail_size()?,
            task_pool_size: stored.int(TASK_POOL_SIZE).unwrap_or(1),
            server_port: stored.int(SERVER_PORT),
            server_context_path: stored.string(SERVER_CONTEXT_PATH),
            kobo_proxy: stored.boolean(KOBO_PROXY).unwrap_or(false),
            kobo_port: stored.int(KOBO_PORT),
            kepubify_path: stored
                .string(KEPUBIFY_PATH)
                .filter(|path| !path.trim().is_empty()),
            next_ui_as_default: stored.boolean(NEXTUI_AS_DEFAULT).unwrap_or(false),
        })
    }
}

#[derive(Clone)]
pub struct ServerSettingsService {
    db: Database,
    task_pool_size: TaskPoolSize,
}

impl ServerSettingsService {
    #[must_use]
    pub fn new(db: Database, task_pool_size: TaskPoolSize) -> Self {
        Self { db, task_pool_size }
    }

    /// # Errors
    /// See [`ServerSettings::load`].
    pub async fn load(&self) -> Result<ServerSettings, SettingsError> {
        ServerSettings::load(&self.db).await
    }

    /// Applies every change in one transaction, so a failure leaves all settings as they were.
    /// A committed `taskPoolSize` resizes the task pool at once.
    ///
    /// # Errors
    /// Fails on database errors or when a new remember-me key cannot be generated.
    pub async fn update(&self, update: ServerSettingsUpdate) -> Result<(), SettingsError> {
        let ServerSettingsUpdate {
            delete_empty_collections,
            delete_empty_read_lists,
            remember_me_duration_days,
            renew_remember_me_key,
            thumbnail_size,
            task_pool_size,
            server_port,
            server_context_path,
            kobo_proxy,
            kobo_port,
            kepubify_path,
            next_ui_as_default,
        } = update;
        let remember_me_key = if renew_remember_me_key {
            Some(generate_remember_me_key()?)
        } else {
            None
        };
        let changes: [(&str, Option<Option<String>>); 12] = [
            (DELETE_EMPTY_COLLECTIONS, set(delete_empty_collections)),
            (DELETE_EMPTY_READLISTS, set(delete_empty_read_lists)),
            (
                REMEMBER_ME_DURATION,
                set(remember_me_duration_days.map(|days| days as i32)),
            ),
            (REMEMBER_ME_KEY, set(remember_me_key)),
            (THUMBNAIL_SIZE, set(thumbnail_size.map(ThumbnailSize::name))),
            (TASK_POOL_SIZE, set(task_pool_size)),
            (SERVER_PORT, set_or_delete(server_port)),
            (SERVER_CONTEXT_PATH, set_or_delete(server_context_path)),
            (KOBO_PROXY, set(kobo_proxy)),
            (KOBO_PORT, set_or_delete(kobo_port)),
            (KEPUBIFY_PATH, set_or_delete(kepubify_path)),
            (NEXTUI_AS_DEFAULT, set(next_ui_as_default)),
        ];

        let mut transaction = self.db.begin_immediate().await?;
        for (key, change) in changes {
            match change {
                Some(Some(value)) => ServerSettingsDao::save(&mut transaction, key, &value).await?,
                Some(None) => ServerSettingsDao::delete(&mut transaction, key).await?,
                None => {}
            }
        }
        transaction.commit().await?;
        if let Some(size) = task_pool_size {
            self.task_pool_size.set(size);
        }
        Ok(())
    }
}

fn set<T: ToString>(value: Option<T>) -> Option<Option<String>> {
    value.map(|value| Some(value.to_string()))
}

fn set_or_delete<T: ToString>(value: Option<Option<T>>) -> Option<Option<String>> {
    value.map(|value| value.map(|value| value.to_string()))
}

/// Stored text values, converted like jOOQ 3.19 `Convert`: unrecognized booleans and integers read
/// as absent, so the setting falls back to its default.
struct Stored(HashMap<String, String>);

impl Stored {
    fn string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }

    fn boolean(&self, key: &str) -> Option<bool> {
        let value = self.0.get(key)?.trim_matches(|character| character <= ' ');
        match value.to_lowercase().as_str() {
            "1" | "1.0" | "y" | "yes" | "true" | "t" | "on" | "enabled" => Some(true),
            "0" | "0.0" | "n" | "no" | "false" | "f" | "off" | "disabled" => Some(false),
            _ => None,
        }
    }

    // jOOQ falls back to `BigDecimal.intValue()` for decimal text; no writer stores such values.
    fn int(&self, key: &str) -> Option<i32> {
        self.0
            .get(key)?
            .trim_matches(|character| character <= ' ')
            .parse()
            .ok()
    }

    /// Kotlin `ThumbnailSize.valueOf` throws on unknown names, which fails startup.
    fn thumbnail_size(&self) -> Result<ThumbnailSize, SettingsError> {
        self.0
            .get(THUMBNAIL_SIZE)
            .map_or(Ok(ThumbnailSize::default()), |value| {
                value
                    .parse()
                    .map_err(|()| SettingsError::InvalidStoredValue {
                        key: THUMBNAIL_SIZE,
                        value: value.clone(),
                    })
            })
    }
}

pub(super) fn generate_remember_me_key() -> Result<String, SettingsError> {
    let mut random_key = [0_u8; 32];
    getrandom::fill(&mut random_key)?;
    Ok(hex::encode(random_key))
}
