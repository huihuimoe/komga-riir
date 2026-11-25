//! The locales names sort by; Komga always uses the JVM default locale.

use std::collections::BTreeMap;

use komga_db::{SortLocale, SortRules};
use komga_types::LibraryId;

use super::property_file::PropertyFiles;
use super::{ConfigError, Environment, Setting, default, invalid, lookup};

const SORT_LOCALE: &str = "komga.sort.locale";
const SORT_LIBRARY_LOCALES: &str = "komga.sort.library-locales";

const UNSUPPORTED_LOCALE: &str = "must be und, a language with its own order such as sv or ja, \
    or zh-u-co-pinyin, zh-u-co-stroke or zh-u-co-zhuyin";

/// The locales names sort by.
#[derive(Debug, Clone)]
pub struct SortConfig {
    /// Orders libraries without their own locale, and results spanning several libraries.
    pub locale: Setting<SortLocale>,
    /// Orders results drawn from one library alone.
    pub library_locales: Setting<BTreeMap<LibraryId, SortLocale>>,
}

impl SortConfig {
    pub fn rules(&self) -> SortRules {
        SortRules::new(
            self.locale.value.clone(),
            self.library_locales.value.clone(),
        )
    }

    pub(super) fn resolve(
        env: &Environment,
        files: &mut PropertyFiles,
    ) -> Result<Self, ConfigError> {
        let locale = match lookup(env, files, SORT_LOCALE)? {
            Some(setting) => match SortLocale::parse(setting.value.trim()) {
                Ok(value) => Setting {
                    value,
                    source: setting.source,
                },
                Err(_) => return Err(invalid(SORT_LOCALE, setting, UNSUPPORTED_LOCALE)),
            },
            None => default(SortLocale::default()),
        };
        let library_locales = match lookup(env, files, SORT_LIBRARY_LOCALES)? {
            Some(setting) => match parse_library_locales(&setting.value) {
                Ok(value) => Setting {
                    value,
                    source: setting.source,
                },
                Err(reason) => return Err(invalid(SORT_LIBRARY_LOCALES, setting, reason)),
            },
            None => default(BTreeMap::new()),
        };
        Ok(Self {
            locale,
            library_locales,
        })
    }
}

/// Parses `<library id>=<locale>` pairs separated by commas.
fn parse_library_locales(value: &str) -> Result<BTreeMap<LibraryId, SortLocale>, &'static str> {
    let mut locales = BTreeMap::new();
    if value.trim().is_empty() {
        return Ok(locales);
    }
    for pair in value.split(',') {
        let (library, locale) = pair
            .split_once('=')
            .map(|(library, locale)| (library.trim(), locale.trim()))
            .filter(|(library, _)| !library.is_empty())
            .ok_or("must list <library id>=<locale> pairs separated by commas")?;
        let locale = SortLocale::parse(locale).map_err(|_| UNSUPPORTED_LOCALE)?;
        if locales.insert(LibraryId::from(library), locale).is_some() {
            return Err("must not name a library more than once");
        }
    }
    Ok(locales)
}
