//! SQLite collations registered on every connection (Kotlin `SqliteUdfDataSource`).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::sync::LazyLock;

use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};
use icu_locale_core::Locale;
use komga_types::{ContentScope, LibraryId};
use sea_query::Expr;
use sqlx::sqlite::SqliteConnectOptions;

/// `COLLATION_UNICODE_1`, used for matching names regardless of case and accents.
pub(crate) const UNICODE_1: &str = "COLLATION_UNICODE_1";

/// Kotlin `Collators.collator1`: primary strength over canonically decomposed text, which ICU4X
/// always applies, so only base letters differ.
static COLLATOR_1: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    let mut options = CollatorOptions::default();
    options.strength = Some(Strength::Primary);
    Collator::try_new(Default::default(), options).expect("Compiled root collation data")
});

pub(crate) fn compare_unicode_1(left: &str, right: &str) -> Ordering {
    COLLATOR_1.compare(left, right)
}

/// Part of every sort collation name. A name must keep its meaning for as long as anything is
/// ordered under it, so bump this when the ICU data or the comparison of [`SortLocale`] changes.
const SORT_RULE_VERSION: u32 = 1;

/// The root order.
const ROOT: &str = "und";

/// Languages whose order differs from the root one in the compiled ICU data.
const TAILORED_LANGUAGES: &[&str] = &[
    "af", "ar", "as", "az", "be", "blo", "bn", "br", "bs", "ceb", "cs", "cy", "da", "dsb", "ee",
    "eo", "es", "et", "fa", "fi", "fil", "fo", "fy", "gl", "gu", "ha", "hi", "hr", "hsb", "hu",
    "hy", "ig", "is", "ja", "kk", "km", "kn", "ko", "kok", "ku", "ky", "lt", "lv", "mk", "ml",
    "mr", "mt", "my", "no", "nso", "om", "or", "pa", "pl", "ps", "ro", "si", "sk", "sl", "sq",
    "sr", "sv", "ta", "te", "th", "tk", "tn", "to", "tr", "ug", "uk", "ur", "uz", "vi", "wo", "yo",
];

/// Chinese has no single order, so its locale must name one.
const CHINESE_ORDERS: [&str; 3] = ["zh-u-co-pinyin", "zh-u-co-stroke", "zh-u-co-zhuyin"];

/// A locale that names sort by.
///
/// Names compare at tertiary strength with runs of digits compared as numbers. Names the locale
/// holds equal, such as `1` and `01` or canonically equivalent spellings, fall back to their
/// code point order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortLocale {
    tag: String,
}

/// The locale is not one of the supported sort locales.
#[derive(Debug, thiserror::Error)]
#[error("unsupported sort locale: {0}")]
pub struct UnsupportedSortLocale(String);

impl SortLocale {
    /// Accepts `und`, a language with its own order, or Chinese with an explicit order
    /// (`zh-u-co-pinyin`, `zh-u-co-stroke`, `zh-u-co-zhuyin`).
    ///
    /// # Errors
    /// Rejects every other tag instead of falling back to a related order.
    pub fn parse(tag: &str) -> Result<Self, UnsupportedSortLocale> {
        if tag == ROOT || TAILORED_LANGUAGES.contains(&tag) || CHINESE_ORDERS.contains(&tag) {
            Ok(Self {
                tag: tag.to_owned(),
            })
        } else {
            Err(UnsupportedSortLocale(tag.to_owned()))
        }
    }

    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    fn collation(&self) -> String {
        let tag = self.tag.replace('-', "_").to_ascii_uppercase();
        format!("ERA_SORT_{SORT_RULE_VERSION}_{tag}")
    }

    fn collator(&self) -> CollatorBorrowed<'static> {
        let locale: Locale = self.tag.parse().expect("Supported tags are well-formed");
        let mut preferences = CollatorPreferences::from(&locale);
        preferences.numeric_ordering = Some(CollationNumericOrdering::True);
        let mut options = CollatorOptions::default();
        options.strength = Some(Strength::Tertiary);
        Collator::try_new(preferences, options).expect("Supported tags have compiled data")
    }
}

impl Default for SortLocale {
    fn default() -> Self {
        Self {
            tag: ROOT.to_owned(),
        }
    }
}

fn compare(collator: &CollatorBorrowed<'static>, left: &str, right: &str) -> Ordering {
    collator.compare(left, right).then_with(|| left.cmp(right))
}

/// The locales names sort by: one per library that sets its own, and a global one for the other
/// libraries and for results spanning several libraries.
#[derive(Debug, Clone, Default)]
pub struct SortRules {
    global: SortLocale,
    libraries: BTreeMap<LibraryId, SortLocale>,
}

impl SortRules {
    #[must_use]
    pub fn new(global: SortLocale, libraries: BTreeMap<LibraryId, SortLocale>) -> Self {
        Self { global, libraries }
    }

    /// The collation ordering the names of a result drawn from `library` alone, or from any
    /// number of libraries when `None`.
    pub(crate) fn collation(&self, library: Option<&str>) -> String {
        library
            .and_then(|library| self.libraries.iter().find(|(id, _)| id.as_str() == library))
            .map_or(&self.global, |(_, locale)| locale)
            .collation()
    }

    /// Registers the collation of every locale in use on the connections of `options`.
    pub(crate) fn register(&self, mut options: SqliteConnectOptions) -> SqliteConnectOptions {
        let mut locales: Vec<&SortLocale> = self.libraries.values().collect();
        locales.push(&self.global);
        locales.sort_by_key(|locale| locale.tag());
        locales.dedup();
        for locale in locales {
            let collator = locale.collator();
            options = options.collation(locale.collation(), move |left, right| {
                compare(&collator, left, right)
            });
        }
        options
    }
}

/// The library a result is drawn from, when `filtered` or else `scope` leaves exactly one.
pub(crate) fn sole_library<'a>(
    scope: &'a ContentScope,
    filtered: Option<&'a str>,
) -> Option<&'a str> {
    filtered.or_else(|| {
        let mut libraries = scope.library_ids()?.iter();
        let first = libraries.next()?;
        libraries.next().is_none().then_some(first.as_str())
    })
}

/// `text COLLATE collation`.
pub(crate) fn collate(text: Expr, collation: &str) -> Expr {
    Expr::cust_with_expr(format!("? COLLATE {collation}"), text)
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use super::{SortLocale, compare, compare_unicode_1};

    fn sorted(tag: &str, mut names: Vec<&'static str>) -> Vec<&'static str> {
        let collator = SortLocale::parse(tag)
            .expect("The tag is supported")
            .collator();
        names.sort_by(|left, right| compare(&collator, left, right));
        names
    }

    #[test]
    fn orders_by_letters_then_accents_then_case_with_numbers_by_value() {
        assert_eq!(
            sorted("und", vec!["b", "Á", "a", "A", "á", "Z", "10", "9"]),
            ["9", "10", "a", "A", "á", "Á", "b", "Z"]
        );
        // Digits compare as integers, not as decimals: 1.10 follows 1.9.
        assert_eq!(
            sorted("und", vec!["v1.10", "v1.9", "v10", "v2"]),
            ["v1.9", "v1.10", "v2", "v10"]
        );
    }

    #[test]
    fn names_the_locale_holds_equal_fall_back_to_code_points() {
        assert_eq!(
            sorted("und", vec!["vol 2", "vol 1", "vol 01", "vol 001"]),
            ["vol 001", "vol 01", "vol 1", "vol 2"]
        );
        assert_eq!(
            sorted("und", vec!["\u{e9}", "e\u{301}"]),
            ["e\u{301}", "\u{e9}"]
        );
    }

    #[test]
    fn locales_tailor_the_order() {
        let names = vec!["z", "ä", "a"];
        assert_eq!(sorted("und", names.clone()), ["a", "ä", "z"]);
        assert_eq!(sorted("sv", names), ["a", "z", "ä"]);
        let names = vec!["一", "中", "二"];
        assert_eq!(sorted("zh-u-co-pinyin", names.clone()), ["二", "一", "中"]);
        assert_eq!(sorted("zh-u-co-stroke", names), ["一", "二", "中"]);
    }

    #[test]
    fn every_supported_locale_has_a_distinct_collation() {
        let tags = super::TAILORED_LANGUAGES
            .iter()
            .copied()
            .chain(super::CHINESE_ORDERS)
            .chain([super::ROOT]);
        let mut collations = Vec::new();
        for tag in tags {
            let locale = SortLocale::parse(tag).expect("The tag is supported");
            let _ = locale.collator();
            collations.push(locale.collation());
        }
        let count = collations.len();
        collations.sort();
        collations.dedup();
        assert_eq!(collations.len(), count);
        assert_eq!(
            SortLocale::parse("zh-u-co-pinyin").unwrap().collation(),
            "ERA_SORT_1_ZH_U_CO_PINYIN"
        );
    }

    #[test]
    fn other_locales_are_rejected() {
        for tag in ["zh", "zh-Hant", "de", "sv-SE", "SV", "zh-u-co-unihan", ""] {
            assert!(SortLocale::parse(tag).is_err(), "{tag}");
        }
    }

    #[test]
    fn primary_strength_ignores_case_and_accents() {
        assert_eq!(compare_unicode_1("École", "ecole"), Ordering::Equal);
        assert_eq!(compare_unicode_1("a", "b"), Ordering::Less);
    }
}
