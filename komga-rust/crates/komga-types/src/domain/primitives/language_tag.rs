//! Kotlin `BCP47TagValidator`, with the IANA Language Subtag Registry in place of ICU's list of
//! ISO languages.

use language_tags::LanguageTag;

/// `isValid` then `normalize`: `value` in canonical case when it is a well-formed BCP 47 tag
/// whose primary language is a registered ISO 639 language, `None` otherwise. Unlike ICU's lenient
/// parser, a tag with an ill-formed subtag is rejected instead of cut before that subtag.
#[must_use]
pub fn normalize_language_tag(value: &str) -> Option<String> {
    let tag = LanguageTag::parse(value).ok()?;
    let language = tag.primary_language();
    // ICU reads no language from grandfathered tags such as `i-klingon`, private use (`qaa`
    // to `qtz`), or `und`.
    let iso_language = (2..=3).contains(&language.len())
        && language.bytes().all(|byte| byte.is_ascii_lowercase())
        && !("qaa"..="qtz").contains(&language)
        && language != "und";
    (iso_language && tag.is_valid()).then(|| tag.into_string())
}

#[cfg(test)]
mod tests {
    use super::normalize_language_tag;

    #[test]
    fn registered_languages_are_kept_in_canonical_case() {
        for (value, normalized) in [
            ("en", "en"),
            ("EN-us", "en-US"),
            ("zh-hant-tw", "zh-Hant-TW"),
            ("ja", "ja"),
        ] {
            assert_eq!(normalize_language_tag(value).as_deref(), Some(normalized));
        }
    }

    #[test]
    fn unknown_or_ill_formed_tags_are_rejected() {
        for value in [
            "",
            " en",
            "en_US",
            "english",
            "xx",
            "qab",
            "und",
            "i-klingon",
            "x-private",
        ] {
            assert_eq!(normalize_language_tag(value), None, "{value:?}");
        }
    }
}
