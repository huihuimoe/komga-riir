//! The baseline `spring.config.import` files in the config directory.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use yaml_rust2::{Yaml, YamlLoader};

use super::{ConfigError, ConfigSource, IgnoredSetting};

/// Imported files, lowest precedence first: a later import overrides an earlier one.
const FILE_NAMES: [&str; 3] = [
    "application.yml",
    "application.yaml",
    "application.properties",
];

/// A file entry, with its key as written.
pub(super) struct FileValue {
    pub key: String,
    pub value: String,
    pub path: PathBuf,
}

/// Effective file entries keyed by their relaxed name. Readers [`take`](Self::take) the keys they
/// understand; anything left over is ignored.
pub(super) struct PropertyFiles {
    values: BTreeMap<String, FileValue>,
}

impl PropertyFiles {
    pub(super) fn load(config_dir: &Path) -> Result<Self, ConfigError> {
        let mut values = BTreeMap::new();
        for name in FILE_NAMES {
            let path = config_dir.join(name);
            let contents = match fs::read(&path) {
                Ok(contents) => contents,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(ConfigError::ReadFile { path, error }),
            };
            let entries = if name.ends_with(".properties") {
                parse_properties(&path, &contents)?
            } else {
                parse_yaml(&path, &contents)?
            };
            values.extend(relaxed_entries(&path, entries)?);
        }
        Ok(Self { values })
    }

    pub(super) fn take(&mut self, key: &str) -> Option<FileValue> {
        self.values.remove(&uniform_key(key))
    }

    /// Takes every entry under a relaxed key prefix such as `spring.security.oauth2.client.`.
    pub(super) fn take_prefixed(&mut self, prefix: &str) -> Vec<FileValue> {
        let prefix = uniform_key(prefix);
        let keys: Vec<String> = self
            .values
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .cloned()
            .collect();
        keys.iter()
            .filter_map(|key| self.values.remove(key))
            .collect()
    }

    /// The entries no reader took, as written.
    pub(super) fn into_remaining(self) -> impl Iterator<Item = IgnoredSetting> {
        self.values.into_values().map(|found| IgnoredSetting {
            key: found.key,
            source: ConfigSource::File(found.path),
        })
    }
}

/// Keys one file spells in several relaxed forms (`configDir`, `config-dir`) must agree.
fn relaxed_entries(
    path: &Path,
    entries: Vec<(String, String)>,
) -> Result<BTreeMap<String, FileValue>, ConfigError> {
    let mut values: BTreeMap<String, FileValue> = BTreeMap::new();
    for (key, value) in entries {
        let uniform = uniform_key(&key);
        if let Some(existing) = values.get(&uniform)
            && existing.value != value
        {
            return Err(ConfigError::FileConflict {
                path: path.to_path_buf(),
                first: existing.key.clone(),
                second: key,
            });
        }
        values.insert(
            uniform,
            FileValue {
                key,
                value,
                path: path.to_path_buf(),
            },
        );
    }
    Ok(values)
}

// Like Java's `Properties.load`, the file is ISO-8859-1 with `\uXXXX` escapes.
fn parse_properties(path: &Path, contents: &[u8]) -> Result<Vec<(String, String)>, ConfigError> {
    java_properties::read(contents)
        .map(|entries| entries.into_iter().collect())
        .map_err(|error| ConfigError::ParseFile {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

fn parse_yaml(path: &Path, contents: &[u8]) -> Result<Vec<(String, String)>, ConfigError> {
    let parse_error = |message: String| ConfigError::ParseFile {
        path: path.to_path_buf(),
        message,
    };
    let text = std::str::from_utf8(contents).map_err(|error| parse_error(error.to_string()))?;
    let documents =
        YamlLoader::load_from_str(text).map_err(|error| parse_error(error.to_string()))?;
    let mut entries = Vec::new();
    match documents.as_slice() {
        [] | [Yaml::Null] => {}
        [Yaml::Hash(root)] => {
            for (key, value) in root {
                flatten(mapping_key(key), value, &mut entries).map_err(parse_error)?;
            }
        }
        [_] => {
            return Err(parse_error(
                "the document root must be a mapping".to_string(),
            ));
        }
        _ => {
            return Err(parse_error(
                "multi-document YAML is not supported".to_string(),
            ));
        }
    }
    Ok(entries)
}

/// Flattens nested mappings into Spring's dotted keys and lists into `key[index]`.
fn flatten(
    key: Result<String, String>,
    node: &Yaml,
    entries: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let key = key?;
    match node {
        Yaml::Hash(hash) => {
            for (child, value) in hash {
                let child = mapping_key(child).map(|child| format!("{key}.{child}"));
                flatten(child, value, entries)?;
            }
        }
        Yaml::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                flatten(Ok(format!("{key}[{index}]")), item, entries)?;
            }
        }
        _ => {
            let value =
                scalar(node).ok_or_else(|| format!("unsupported value {node:?} for {key}"))?;
            entries.push((key, value));
        }
    }
    Ok(())
}

fn mapping_key(node: &Yaml) -> Result<String, String> {
    scalar(node).ok_or_else(|| format!("unsupported mapping key {node:?}"))
}

/// Spring reads an empty YAML value as an empty string.
fn scalar(node: &Yaml) -> Option<String> {
    match node {
        Yaml::String(value) | Yaml::Real(value) => Some(value.clone()),
        Yaml::Integer(value) => Some(value.to_string()),
        Yaml::Boolean(value) => Some(value.to_string()),
        Yaml::Null => Some(String::new()),
        Yaml::Array(_) | Yaml::Hash(_) | Yaml::Alias(_) | Yaml::BadValue => None,
    }
}

/// Spring's relaxed property-name form: `configDir`, `config_dir`, and `config-dir` are the same
/// key, so only lowercase alphanumerics, dots, and list indices are compared.
fn uniform_key(key: &str) -> String {
    key.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '[' | ']'))
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}
