//! `KomgaProperties.Lucene`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use komga_search::NgramOptions;

use super::property_file::PropertyFiles;
use super::{
    ConfigError, Environment, Setting, default, duration, lookup, parse_boolean,
    parse_positive_int, resolve_path,
};

const LUCENE_DATA_DIRECTORY: &str = "komga.lucene.data-directory";
const LUCENE_MIN_GRAM: &str = "komga.lucene.index-analyzer.min-gram";
const LUCENE_MAX_GRAM: &str = "komga.lucene.index-analyzer.max-gram";
const LUCENE_PRESERVE_ORIGINAL: &str = "komga.lucene.index-analyzer.preserve-original";
const LUCENE_COMMIT_DELAY: &str = "komga.lucene.commit-delay";

const DEFAULT_COMMIT_DELAY: Duration = Duration::from_secs(2);

/// The full-text index (`KomgaProperties.Lucene`).
#[derive(Debug, Clone)]
pub struct LuceneConfig {
    /// Holds the index; Komga's Lucene files there are left alone.
    pub data_directory: Setting<PathBuf>,
    /// N-gram lengths indexed for each term; at least 1, and `max_gram` at least `min_gram`.
    pub min_gram: Setting<usize>,
    pub max_gram: Setting<usize>,
    /// Also index terms shorter or longer than every n-gram.
    pub preserve_original: Setting<bool>,
    /// How long changes gather before the index commits them.
    pub commit_delay: Setting<Duration>,
}

impl LuceneConfig {
    pub fn ngram(&self) -> NgramOptions {
        NgramOptions::new(
            self.min_gram.value,
            self.max_gram.value,
            self.preserve_original.value,
        )
        .expect("the gram lengths are validated when resolved")
    }

    pub(super) fn resolve(
        env: &Environment,
        files: &mut PropertyFiles,
        config_dir: &Path,
    ) -> Result<Self, ConfigError> {
        let data_directory = match lookup(env, files, LUCENE_DATA_DIRECTORY)? {
            Some(directory) => resolve_path(LUCENE_DATA_DIRECTORY, directory, config_dir, env)?,
            None => default(config_dir.join("lucene")),
        };
        let gram = |files: &mut PropertyFiles, key, default_value| {
            lookup(env, files, key)?.map_or(Ok(default(default_value)), |setting| {
                let setting = parse_positive_int(key, setting)?;
                Ok(Setting {
                    value: usize::try_from(setting.value).expect("a positive i32 fits in usize"),
                    source: setting.source,
                })
            })
        };
        let min_gram = gram(files, LUCENE_MIN_GRAM, 3)?;
        let max_gram = gram(files, LUCENE_MAX_GRAM, 10)?;
        if max_gram.value < min_gram.value {
            return Err(ConfigError::InvalidValue {
                key: LUCENE_MAX_GRAM,
                value: max_gram.value.to_string(),
                origin: max_gram.source,
                reason: "must not be less than komga.lucene.index-analyzer.min-gram",
            });
        }
        let preserve_original = lookup(env, files, LUCENE_PRESERVE_ORIGINAL)?
            .map_or(Ok(default(true)), |setting| {
                parse_boolean(LUCENE_PRESERVE_ORIGINAL, setting)
            })?;
        let commit_delay = lookup(env, files, LUCENE_COMMIT_DELAY)?.map_or(
            Ok(default(DEFAULT_COMMIT_DELAY)),
            |setting| {
                duration::parse_duration(LUCENE_COMMIT_DELAY, setting, duration::MILLISECONDS)
            },
        )?;
        Ok(Self {
            data_directory,
            min_gram,
            max_gram,
            preserve_original,
            commit_delay,
        })
    }
}
