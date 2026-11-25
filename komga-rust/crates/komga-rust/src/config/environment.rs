//! Process environment variables under the baseline Spring Boot names.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use super::{ConfigError, ConfigSource, IgnoredSetting, Setting};

/// Baseline keys (`KomgaProperties`, `application.yml`, and the Spring Boot properties Komga reads)
/// that komga-rust does not honor. Other variables are not configuration at all.
const UNSUPPORTED_KEYS: &[&str] = &[
    "komga.jdbc-session-enabled",
    "komga.workspace",
    "komga.file-hashing",
    "komga.libraries-scan-startup",
    "komga.delete-empty-collections",
    "komga.delete-empty-read-lists",
    "komga.cors.allowed-origins",
    "komga.kobo.kepubify-path",
    "komga.database.batch-chunk-size",
    "komga.database.pool-size",
    "komga.database.max-pool-size",
    "komga.database.journal-mode",
    "komga.database.busy-timeout",
    "komga.tasks-db.batch-chunk-size",
    "komga.tasks-db.pool-size",
    "komga.tasks-db.max-pool-size",
    "komga.tasks-db.journal-mode",
    "komga.tasks-db.busy-timeout",
    "komga.sessions-db.file",
    "komga.sessions-db.batch-chunk-size",
    "komga.sessions-db.pool-size",
    "komga.sessions-db.max-pool-size",
    "komga.sessions-db.journal-mode",
    "komga.sessions-db.busy-timeout",
    "komga.sessions-db.check-local-filesystem",
    "spring.servlet.multipart.max-file-size",
    "server.servlet.session.timeout",
    "spring.profiles.active",
    "spring.profiles.include",
];

/// Variable prefixes of baseline map-valued keys (`komga.*.pragmas`).
const UNSUPPORTED_PREFIXES: &[&str] = &[
    "KOMGA_DATABASE_PRAGMAS_",
    "KOMGA_TASKSDB_PRAGMAS_",
    "KOMGA_SESSIONSDB_PRAGMAS_",
];

/// A snapshot of environment variables.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    variables: BTreeMap<String, OsString>,
}

impl Environment {
    /// Captures the current process environment. Variables with non-Unicode names are skipped
    /// because no baseline name contains such characters.
    pub fn from_process() -> Self {
        std::env::vars_os()
            .filter_map(|(name, value)| name.into_string().ok().map(|name| (name, value)))
            .collect()
    }

    /// Returns the variable for `key`, accepting both names the baseline honors: Spring's binding
    /// name (`KOMGA_CONFIGDIR`) and its placeholder name (`KOMGA_CONFIG_DIR`).
    pub(super) fn lookup(&self, key: &'static str) -> Result<Option<Setting<String>>, ConfigError> {
        let mut found: Option<(String, String)> = None;
        for name in variable_names(key) {
            let Some(value) = self.string(&name)? else {
                continue;
            };
            match &found {
                Some((first, first_value)) if *first_value != value => {
                    return Err(ConfigError::EnvConflict {
                        key,
                        first: first.clone(),
                        second: name,
                    });
                }
                Some(_) => {}
                None => found = Some((name, value)),
            }
        }
        Ok(found.map(|(name, value)| Setting {
            value,
            source: ConfigSource::Env(name),
        }))
    }

    pub(super) fn home_dir(&self) -> Option<PathBuf> {
        ["HOME", "USERPROFILE"]
            .into_iter()
            .filter_map(|name| self.variables.get(name))
            .find(|value| !value.is_empty())
            .map(PathBuf::from)
    }

    pub(super) fn unsupported(&self) -> Vec<IgnoredSetting> {
        let keyed = UNSUPPORTED_KEYS.iter().flat_map(|key| {
            variable_names(key)
                .into_iter()
                .filter(|name| self.variables.contains_key(name))
                .map(|name| ((*key).to_owned(), name))
        });
        let prefixed = self
            .variables
            .keys()
            .filter(|name| {
                UNSUPPORTED_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            })
            .map(|name| (name.clone(), name.clone()));
        keyed
            .chain(prefixed)
            .map(|(key, name)| IgnoredSetting {
                key,
                source: ConfigSource::Env(name),
            })
            .collect()
    }

    /// The variables whose names start with `prefix`, in name order.
    pub(super) fn prefixed<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = (String, Result<String, ConfigError>)> + 'a {
        self.variables
            .keys()
            .filter(move |name| name.starts_with(prefix))
            .map(|name| {
                (
                    name.clone(),
                    self.string(name).map(Option::unwrap_or_default),
                )
            })
    }

    fn string(&self, name: &str) -> Result<Option<String>, ConfigError> {
        self.variables
            .get(name)
            .map(|value| {
                value
                    .clone()
                    .into_string()
                    .map_err(|_| ConfigError::NonUnicodeEnv {
                        name: name.to_string(),
                    })
            })
            .transpose()
    }
}

impl<N: Into<String>, V: Into<OsString>> FromIterator<(N, V)> for Environment {
    fn from_iter<I: IntoIterator<Item = (N, V)>>(iter: I) -> Self {
        Self {
            variables: iter
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        }
    }
}

/// Spring binds `komga.config-dir` from `KOMGA_CONFIGDIR` (dashes dropped) and resolves the
/// `${komga.config-dir}` placeholder from `KOMGA_CONFIG_DIR` (dashes as underscores).
fn variable_names(key: &str) -> Vec<String> {
    let binding = key
        .split('.')
        .map(|segment| segment.replace('-', "").to_ascii_uppercase())
        .collect::<Vec<String>>()
        .join("_");
    let placeholder = key.replace(['.', '-'], "_").to_ascii_uppercase();
    if binding == placeholder {
        vec![binding]
    } else {
        vec![binding, placeholder]
    }
}
