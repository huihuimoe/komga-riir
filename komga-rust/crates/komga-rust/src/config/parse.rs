//! Configuration value parsing, path resolution, and placeholder expansion helpers.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::property_file::PropertyFiles;
use super::{
    CONFIG_DIR, CONTEXT_PATH, CliArgs, ConfigError, ConfigSource, DATABASE_CHECK_LOCAL_FILESYSTEM,
    DATABASE_FILE, Environment, FONTS_DATA_DIRECTORY, LOG_FILE, SERVER_PORT, Setting,
    TASKS_DB_CHECK_LOCAL_FILESYSTEM, TASKS_DB_FILE,
};

pub(super) struct StorageSettings {
    pub(super) database_file: Setting<PathBuf>,
    pub(super) check_local_filesystem: Setting<bool>,
    pub(super) tasks_database_file: Setting<PathBuf>,
    pub(super) tasks_check_local_filesystem: Setting<bool>,
    pub(super) fonts_data_directory: Setting<PathBuf>,
    pub(super) log_file: Setting<PathBuf>,
}

pub(super) fn resolve_main_database(
    config_dir: &Path,
    env: &Environment,
    files: &mut PropertyFiles,
) -> Result<(Setting<PathBuf>, Setting<bool>), ConfigError> {
    let database_file = resolve_path_or(
        env,
        files,
        config_dir,
        DATABASE_FILE,
        config_dir.join("database.sqlite"),
    )?;
    let check_local_filesystem = lookup(env, files, DATABASE_CHECK_LOCAL_FILESYSTEM)?
        .map_or(Ok(default(true)), |setting| {
            parse_boolean(DATABASE_CHECK_LOCAL_FILESYSTEM, setting)
        })?;
    Ok((database_file, check_local_filesystem))
}

pub(super) fn resolve_storage(
    config_dir: &Path,
    env: &Environment,
    files: &mut PropertyFiles,
) -> Result<StorageSettings, ConfigError> {
    let (database_file, check_local_filesystem) = resolve_main_database(config_dir, env, files)?;
    let tasks_database_file = resolve_path_or(
        env,
        files,
        config_dir,
        TASKS_DB_FILE,
        config_dir.join("tasks.sqlite"),
    )?;
    let tasks_check_local_filesystem = lookup(env, files, TASKS_DB_CHECK_LOCAL_FILESYSTEM)?
        .map_or(Ok(default(true)), |setting| {
            parse_boolean(TASKS_DB_CHECK_LOCAL_FILESYSTEM, setting)
        })?;
    let fonts_data_directory = resolve_path_or(
        env,
        files,
        config_dir,
        FONTS_DATA_DIRECTORY,
        config_dir.join("fonts"),
    )?;
    let log_file = resolve_path_or(
        env,
        files,
        config_dir,
        LOG_FILE,
        config_dir.join("logs").join("komga.log"),
    )?;
    Ok(StorageSettings {
        database_file,
        check_local_filesystem,
        tasks_database_file,
        tasks_check_local_filesystem,
        fonts_data_directory,
        log_file,
    })
}

pub(super) fn resolve_config_dir(
    args: &CliArgs,
    env: &Environment,
) -> Result<Setting<PathBuf>, ConfigError> {
    if let Some(dir) = &args.config_dir {
        return Ok(Setting {
            value: dir.clone(),
            source: ConfigSource::Cli("--config-dir"),
        });
    }
    match env.lookup(CONFIG_DIR)? {
        Some(dir) => {
            let dir = not_blank(CONFIG_DIR, dir)?;
            if dir.value.contains("${") {
                return Err(invalid(
                    CONFIG_DIR,
                    dir,
                    "placeholders are not supported here",
                ));
            }
            Ok(Setting {
                value: PathBuf::from(dir.value),
                source: dir.source,
            })
        }
        None => env
            .home_dir()
            .map(|home| default(home.join(".komga")))
            .ok_or(ConfigError::NoHomeDirectory),
    }
}

/// Looks a key up in the environment, then in the files. The file entry is consumed either way so
/// that it does not count as an unsupported leftover.
pub(super) fn lookup(
    env: &Environment,
    files: &mut PropertyFiles,
    key: &'static str,
) -> Result<Option<Setting<String>>, ConfigError> {
    let from_file = files.take(key);
    if let Some(found) = env.lookup(key)? {
        return Ok(Some(found));
    }
    Ok(from_file.map(|found| Setting {
        value: found.value,
        source: ConfigSource::File(found.path),
    }))
}

pub(super) fn cli_setting(option: &'static str, value: Option<String>) -> Option<Setting<String>> {
    value.map(|value| Setting {
        value,
        source: ConfigSource::Cli(option),
    })
}

pub(super) fn default<T>(value: T) -> Setting<T> {
    Setting {
        value,
        source: ConfigSource::Default,
    }
}

pub(super) fn invalid(
    key: &'static str,
    setting: Setting<String>,
    reason: &'static str,
) -> ConfigError {
    ConfigError::InvalidValue {
        key,
        value: setting.value,
        origin: setting.source,
        reason,
    }
}

pub(super) fn not_blank(
    key: &'static str,
    setting: Setting<String>,
) -> Result<Setting<String>, ConfigError> {
    if setting.value.trim().is_empty() {
        return Err(invalid(key, setting, "must not be blank"));
    }
    Ok(setting)
}

/// Stricter than Spring, which takes any path that starts and does not end with `/`: the
/// stored setting's pattern keeps it a plain route prefix.
pub(super) fn parse_context_path(setting: Setting<String>) -> Result<Setting<String>, ConfigError> {
    if komga_types::is_context_path(&setting.value) {
        return Ok(setting);
    }
    Err(invalid(
        CONTEXT_PATH,
        setting,
        "expected / followed by letters, digits, _, - or /, ending with a letter or digit",
    ))
}

// Port 0 (a random port in Spring) is rejected: clients and the server-port setting need a
// predictable address.
pub(super) fn parse_port(setting: Setting<String>) -> Result<Setting<u16>, ConfigError> {
    match setting.value.trim().parse::<u16>() {
        Ok(port) if port != 0 => Ok(Setting {
            value: port,
            source: setting.source,
        }),
        _ => Err(invalid(
            SERVER_PORT,
            setting,
            "expected a port between 1 and 65535",
        )),
    }
}

/// Spring's `StringToNumberConverterFactory` for an `Int`, in decimal.
pub(super) fn parse_int(
    key: &'static str,
    setting: Setting<String>,
) -> Result<Setting<i32>, ConfigError> {
    match setting.value.trim().parse::<i32>() {
        Ok(value) => Ok(Setting {
            value,
            source: setting.source,
        }),
        Err(_) => Err(invalid(key, setting, "expected an integer")),
    }
}

/// [`parse_int`] for a Bean Validation `@Positive` property.
pub(super) fn parse_positive_int(
    key: &'static str,
    setting: Setting<String>,
) -> Result<Setting<i32>, ConfigError> {
    if setting
        .value
        .trim()
        .parse::<i32>()
        .is_ok_and(|value| value <= 0)
    {
        return Err(invalid(key, setting, "must be greater than 0"));
    }
    parse_int(key, setting)
}

/// Spring's `StringToBooleanConverter`: `true`/`on`/`yes`/`1` or `false`/`off`/`no`/`0`, trimmed
/// and case-insensitive.
pub(super) fn parse_boolean(
    key: &'static str,
    setting: Setting<String>,
) -> Result<Setting<bool>, ConfigError> {
    let value = match setting.value.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => true,
        "false" | "off" | "no" | "0" => false,
        _ => return Err(invalid(key, setting, "expected true or false")),
    };
    Ok(Setting {
        value,
        source: setting.source,
    })
}

pub(super) fn resolve_path_or(
    env: &Environment,
    files: &mut PropertyFiles,
    config_dir: &Path,
    key: &'static str,
    fallback: PathBuf,
) -> Result<Setting<PathBuf>, ConfigError> {
    match lookup(env, files, key)? {
        Some(file) => resolve_path(key, file, config_dir, env),
        None => Ok(default(fallback)),
    }
}

pub(super) fn resolve_path(
    key: &'static str,
    setting: Setting<String>,
    config_dir: &Path,
    env: &Environment,
) -> Result<Setting<PathBuf>, ConfigError> {
    let setting = not_blank(key, setting)?;
    match expand_placeholders(&setting.value, config_dir, env.home_dir().as_deref()) {
        Ok(path) => Ok(Setting {
            value: path,
            source: setting.source,
        }),
        Err(reason) => Err(invalid(key, setting, reason)),
    }
}

/// Expands the `${komga.config-dir}` and `${user.home}` placeholders used by the baseline path
/// defaults. Other Spring placeholder forms are rejected rather than kept literally.
fn expand_placeholders(
    value: &str,
    config_dir: &Path,
    home: Option<&Path>,
) -> Result<PathBuf, &'static str> {
    let mut expanded = OsString::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        expanded.push(&rest[..start]);
        let Some(length) = rest[start..].find('}') else {
            return Err("unterminated placeholder");
        };
        let name = &rest[start + 2..start + length];
        if name == CONFIG_DIR {
            expanded.push(config_dir);
        } else if name == "user.home" {
            expanded.push(home.ok_or("${user.home} is used but the home directory is unknown")?);
        } else {
            return Err("only ${komga.config-dir} and ${user.home} placeholders are supported");
        }
        rest = &rest[start + length + 1..];
    }
    expanded.push(rest);
    Ok(PathBuf::from(expanded))
}
