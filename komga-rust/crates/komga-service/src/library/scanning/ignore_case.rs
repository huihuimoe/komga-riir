//! Kotlin and Java case-insensitive comparisons used by the scan: `contains(ignoreCase = true)`
//! for exclusions and the oneshots directory, `equals(ignoreCase = true)` and `IGNORE_CASE`
//! regular expressions for sidecar names.

use icu_casemap::CaseMapperBorrowed;

/// Compares UTF-16 units like Kotlin `Char.equals(other, ignoreCase = true)`; an empty `needle`
/// is contained in every string.
pub(super) fn contains(haystack: &str, needle: &str) -> bool {
    let haystack: Vec<u16> = haystack.encode_utf16().collect();
    let needle: Vec<u16> = needle.encode_utf16().collect();
    needle.is_empty()
        || haystack.windows(needle.len()).any(|window| {
            window
                .iter()
                .zip(&needle)
                .all(|(&left, &right)| unit_equals_ignore_case(left, right))
        })
}

/// Kotlin `String.equals(other, ignoreCase = true)`.
pub(super) fn equals(left: &str, right: &str) -> bool {
    left.encode_utf16().count() == right.encode_utf16().count()
        && left
            .encode_utf16()
            .zip(right.encode_utf16())
            .all(|(left, right)| unit_equals_ignore_case(left, right))
}

/// One character of a Kotlin `IGNORE_CASE` regular expression, which Java matches with
/// `UNICODE_CASE`: the lowercase of the uppercase of both sides agree.
pub(super) fn chars_equal(left: char, right: char) -> bool {
    let mapper = CaseMapperBorrowed::new();
    let fold = |character| mapper.simple_lowercase(mapper.simple_uppercase(character));
    left == right || fold(left) == fold(right)
}

/// Kotlin `Char.equals(other, ignoreCase = true)`: equal uppercase, or equal lowercase of the
/// uppercase, which the latter implies.
fn unit_equals_ignore_case(left: u16, right: u16) -> bool {
    match (
        char::from_u32(u32::from(left)),
        char::from_u32(u32::from(right)),
    ) {
        (Some(left), Some(right)) => chars_equal(left, right),
        // Surrogate halves have no case mapping.
        _ => left == right,
    }
}

#[cfg(test)]
mod tests {
    use super::{contains, equals};

    #[test]
    fn matches_like_kotlin_contains_ignore_case() {
        assert!(contains("/library/Comics/@eaDir/x", "@EADIR"));
        assert!(contains("/library/One-Shots", "one-shots"));
        assert!(contains("/library/any", ""));
        // Kelvin sign: its uppercase is itself, and its lowercase is `k`.
        assert!(contains("/library/\u{212a}", "k"));
        assert!(!contains("/library/series", "series/x"));
    }

    #[test]
    fn equals_like_java_equals_ignore_case() {
        assert!(equals("SeRiEs.JSON", "series.json"));
        // Long s and dotless i uppercase to `S` and `I`.
        assert!(equals("\u{17f}er\u{131}es.json", "series.json"));
        assert!(!equals("series.json ", "series.json"));
        // No full case folding: `ß` never equals `ss`.
        assert!(!equals("\u{df}", "ss"));
    }
}
