//! Static startup configuration.
//!
//! Each setting resolves from the command line, then the baseline environment variables, then the
//! baseline `application.{yml,yaml,properties}` files in the config directory, then its default.
//! Baseline keys that komga-rust does not honor are ignored, and startup warns about each.

mod cli;
mod duration;
mod environment;
mod lucene;
mod oauth2;
mod parse;
mod property_file;
mod sort;

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

pub use cli::{AdminCommand, CliArgs, PasswordReset};
pub use environment::Environment;
pub use lucene::LuceneConfig;
pub use oauth2::OAuth2Config;
use parse::{
    cli_setting, default, invalid, lookup, not_blank, parse_boolean, parse_context_path, parse_int,
    parse_port, parse_positive_int, resolve_config_dir, resolve_main_database, resolve_path,
    resolve_storage,
};
use property_file::PropertyFiles;
pub use sort::SortConfig;

const CONFIG_DIR: &str = "komga.config-dir";
const DATABASE_FILE: &str = "komga.database.file";
pub(crate) const DATABASE_CHECK_LOCAL_FILESYSTEM: &str = "komga.database.check-local-filesystem";
const TASKS_DB_FILE: &str = "komga.tasks-db.file";
pub(crate) const TASKS_DB_CHECK_LOCAL_FILESYSTEM: &str = "komga.tasks-db.check-local-filesystem";
const SERVER_ADDRESS: &str = "server.address";
const SERVER_PORT: &str = "server.port";
const CONTEXT_PATH: &str = "server.servlet.context-path";
const TRUSTED_PROXIES: &str = "komga.server.trusted-proxies";
const FONTS_DATA_DIRECTORY: &str = "komga.fonts.data-directory";
const LOG_FILE: &str = "logging.file.name";
const EPUB_DIVINA_LETTER_COUNT_THRESHOLD: &str = "komga.epub-divina-letter-count-threshold";
const PAGE_HASHING: &str = "komga.page-hashing";
const KOBO_SYNC_ITEM_LIMIT: &str = "komga.kobo.sync-item-limit";
const SESSION_TIMEOUT: &str = "spring.session.timeout";

const DEFAULT_PORT: u16 = 25600;
const DEFAULT_EPUB_DIVINA_LETTER_COUNT_THRESHOLD: i32 = 15;
const DEFAULT_PAGE_HASHING: i32 = 3;
const DEFAULT_KOBO_SYNC_ITEM_LIMIT: i32 = 100;

/// Main database settings resolved for one-shot admin CLI commands.
#[derive(Debug, Clone)]
pub struct AdminConfig {
    pub database_file: PathBuf,
    /// Refuse to touch a database on a `cifs`/`nfs` filesystem.
    pub check_local_filesystem: bool,
}

impl AdminConfig {
    /// Resolves only the main database settings, ignoring unrelated server configuration so
    /// recovery commands keep working even when other settings are misconfigured.
    ///
    /// # Errors
    /// Returns an error if the config directory, config files, or main database settings are
    /// invalid.
    pub fn resolve(args: &CliArgs, env: &Environment) -> Result<Self, ConfigError> {
        let (config_dir, mut files) = load_config_files(args, env)?;
        let (database_file, check_local_filesystem) =
            resolve_main_database(&config_dir.value, env, &mut files)?;
        Ok(Self {
            database_file: database_file.value,
            check_local_filesystem: check_local_filesystem.value,
        })
    }
}

fn load_config_files(
    args: &CliArgs,
    env: &Environment,
) -> Result<(Setting<PathBuf>, PropertyFiles), ConfigError> {
    let config_dir = resolve_config_dir(args, env)?;
    let mut files = PropertyFiles::load(&config_dir.value)?;
    if let Some(found) = files.take(CONFIG_DIR) {
        return Err(ConfigError::ConfigDirInFile {
            key: found.key,
            path: found.path,
        });
    }
    Ok((config_dir, files))
}

/// A baseline or unknown key that is set but has no effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredSetting {
    pub key: String,
    pub source: ConfigSource,
}

/// Where an effective setting came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// A command line option, such as `--port`.
    Cli(&'static str),
    /// An environment variable, by name.
    Env(String),
    /// A configuration file in the config directory.
    File(PathBuf),
    Default,
}

impl fmt::Display for ConfigSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cli(option) => write!(formatter, "command line option {option}"),
            Self::Env(name) => write!(formatter, "environment variable {name}"),
            Self::File(path) => write!(formatter, "file {}", path.display()),
            Self::Default => formatter.write_str("default"),
        }
    }
}

/// An effective setting value together with its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting<T> {
    pub value: T,
    pub source: ConfigSource,
}

/// Resolved startup configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub config_dir: Setting<PathBuf>,
    pub database_file: Setting<PathBuf>,
    /// Refuse to start when the database is on a `cifs`/`nfs` filesystem.
    pub check_local_filesystem: Setting<bool>,
    pub tasks_database_file: Setting<PathBuf>,
    pub tasks_check_local_filesystem: Setting<bool>,
    /// One directory per font family offered to the EPUB reader, besides the embedded ones.
    pub fonts_data_directory: Setting<PathBuf>,
    /// `logging.file.name`: the rolling log file written besides standard output.
    pub log_file: Setting<PathBuf>,
    /// `server.address`; unset binds dual-stack `[::]`, falling back to `0.0.0.0`.
    pub host: Option<Setting<String>>,
    pub port: Setting<u16>,
    /// `server.servlet.context-path`; unset or empty serves from the root.
    pub context_path: Option<Setting<String>>,
    /// `komga.server.trusted-proxies`: peer IP/CIDR allowlist for forwarded headers.
    pub trusted_proxies: Setting<komga_api::TrustedProxies>,
    /// EPUB spine documents with more body text than this make a book incompatible with DiViNa.
    pub epub_divina_letter_count_threshold: Setting<i32>,
    /// How many pages are hashed from each end of a book.
    pub page_hashing: Setting<i32>,
    /// The most books and read lists one Kobo sync response hands out.
    pub kobo_sync_item_limit: Setting<i32>,
    /// `spring.session.timeout`: how long a session lives after its last access, and the session
    /// cookie's `Max-Age`.
    pub session_timeout: Setting<Duration>,
    /// The Kobo store requests are forwarded to when its proxy is on. Not configurable, as in
    /// Kotlin; tests point it at a local store.
    pub kobo_store_url: String,
    /// How long the Kobo store may take to connect, and between reads.
    pub kobo_store_timeout: Duration,
    /// The Komga website's news feed; not configurable, as in Kotlin.
    pub announcements_url: String,
    /// GitHub's list of the latest komga-riir releases.
    pub releases_url: String,
    /// How long one announcements or releases fetch may take.
    pub server_info_timeout: Duration,
    pub lucene: LuceneConfig,
    pub sort: SortConfig,
    pub oauth2: OAuth2Config,
    /// Keys that were set but are not honored, in source order.
    pub ignored: Vec<IgnoredSetting>,
}

/// A static configuration error; startup must not continue.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid value {value:?} for {key} from {origin}: {reason}")]
    InvalidValue {
        key: &'static str,
        value: String,
        origin: ConfigSource,
        reason: &'static str,
    },
    #[error("environment variables {first} and {second} set different values for {key}")]
    EnvConflict {
        key: &'static str,
        first: String,
        second: String,
    },
    #[error("{} sets both {first} and {second} to different values", path.display())]
    FileConflict {
        path: PathBuf,
        first: String,
        second: String,
    },
    #[error("{key} from {origin} is not supported by komga-rust")]
    Unsupported { key: String, origin: ConfigSource },
    #[error(
        "{key} cannot be set in {}: this file is read from the config directory",
        path.display()
    )]
    ConfigDirInFile { key: String, path: PathBuf },
    #[error("environment variable {name} is not valid Unicode")]
    NonUnicodeEnv { name: String },
    #[error("failed to read {}: {error}", path.display())]
    ReadFile {
        path: PathBuf,
        error: std::io::Error,
    },
    #[error("failed to parse {}: {message}", path.display())]
    ParseFile { path: PathBuf, message: String },
    #[error(
        "cannot determine the home directory for the default {CONFIG_DIR}; \
         set --config-dir or KOMGA_CONFIGDIR"
    )]
    NoHomeDirectory,
}

impl AppConfig {
    /// Resolves the effective configuration from all static sources.
    ///
    /// # Errors
    /// Returns an error for invalid values, conflicting environment variables or file keys,
    /// unreadable files, and OAuth2 client properties that komga-rust does not support.
    pub fn resolve(args: &CliArgs, env: &Environment) -> Result<Self, ConfigError> {
        let mut ignored = env.unsupported();
        let (config_dir, mut files) = load_config_files(args, env)?;

        let server = Self::resolve_server(args, env, &mut files)?;
        let storage = resolve_storage(&config_dir.value, env, &mut files)?;
        let epub_divina_letter_count_threshold =
            lookup(env, &mut files, EPUB_DIVINA_LETTER_COUNT_THRESHOLD)?.map_or(
                Ok(default(DEFAULT_EPUB_DIVINA_LETTER_COUNT_THRESHOLD)),
                |setting| parse_int(EPUB_DIVINA_LETTER_COUNT_THRESHOLD, setting),
            )?;
        let page_hashing = lookup(env, &mut files, PAGE_HASHING)?
            .map_or(Ok(default(DEFAULT_PAGE_HASHING)), |setting| {
                parse_positive_int(PAGE_HASHING, setting)
            })?;
        let kobo_sync_item_limit = lookup(env, &mut files, KOBO_SYNC_ITEM_LIMIT)?
            .map_or(Ok(default(DEFAULT_KOBO_SYNC_ITEM_LIMIT)), |setting| {
                parse_positive_int(KOBO_SYNC_ITEM_LIMIT, setting)
            })?;
        let session_timeout = lookup(env, &mut files, SESSION_TIMEOUT)?.map_or(
            Ok(default(komga_service::DEFAULT_SESSION_TTL)),
            parse_session_timeout,
        )?;
        let lucene = LuceneConfig::resolve(env, &mut files, &config_dir.value)?;
        let sort = SortConfig::resolve(env, &mut files)?;
        let oauth2 = oauth2::resolve(env, &mut files)?;
        ignored.extend(files.into_remaining());

        Ok(Self {
            config_dir,
            database_file: storage.database_file,
            check_local_filesystem: storage.check_local_filesystem,
            tasks_database_file: storage.tasks_database_file,
            tasks_check_local_filesystem: storage.tasks_check_local_filesystem,
            fonts_data_directory: storage.fonts_data_directory,
            log_file: storage.log_file,
            host: server.host,
            port: server.port,
            context_path: server.context_path,
            trusted_proxies: server.trusted_proxies,
            epub_divina_letter_count_threshold,
            page_hashing,
            kobo_sync_item_limit,
            session_timeout,
            kobo_store_url: "https://storeapi.kobo.com".to_owned(),
            kobo_store_timeout: Duration::from_secs(60),
            announcements_url: "https://komga.org/blog/feed.json".to_owned(),
            releases_url: "https://api.github.com/repos/huihuimoe/komga-riir/releases?per_page=20"
                .to_owned(),
            server_info_timeout: Duration::from_secs(30),
            lucene,
            sort,
            oauth2,
            ignored,
        })
    }

    fn resolve_server(
        args: &CliArgs,
        env: &Environment,
        files: &mut PropertyFiles,
    ) -> Result<ServerSettings, ConfigError> {
        // Lower layers are consulted even when the CLI wins: file entries must be consumed and
        // conflicting environment variables still fail.
        let port = cli_setting("--port", args.port.map(|port| port.to_string()))
            .or(lookup(env, files, SERVER_PORT)?)
            .map_or(Ok(default(DEFAULT_PORT)), parse_port)?;
        let host = cli_setting("--host", args.host.clone())
            .or(lookup(env, files, SERVER_ADDRESS)?)
            .map(|host| not_blank(SERVER_ADDRESS, host))
            .transpose()?;
        let context_path = lookup(env, files, CONTEXT_PATH)?
            .filter(|path| !path.value.is_empty())
            .map(parse_context_path)
            .transpose()?;
        let trusted_proxies = match lookup(env, files, TRUSTED_PROXIES)? {
            Some(setting) => match komga_api::TrustedProxies::parse(&setting.value) {
                Ok(value) => Setting {
                    value,
                    source: setting.source,
                },
                Err(_) => {
                    return Err(invalid(
                        TRUSTED_PROXIES,
                        setting,
                        "expected comma-separated IPs/CIDRs or one of: none, private, all",
                    ));
                }
            },
            None => default(komga_api::TrustedProxies::default()),
        };
        Ok(ServerSettings {
            port,
            host,
            context_path,
            trusted_proxies,
        })
    }
}

/// Spring accepts zero (every session expires at once) and negative (sessions never expire); both
/// are rejected here.
fn parse_session_timeout(setting: Setting<String>) -> Result<Setting<Duration>, ConfigError> {
    let parsed = duration::parse_duration(SESSION_TIMEOUT, setting.clone(), duration::SECONDS)?;
    if parsed.value.is_zero() {
        return Err(invalid(SESSION_TIMEOUT, setting, "must be positive"));
    }
    Ok(parsed)
}

struct ServerSettings {
    port: Setting<u16>,
    host: Option<Setting<String>>,
    context_path: Option<Setting<String>>,
    trusted_proxies: Setting<komga_api::TrustedProxies>,
}
