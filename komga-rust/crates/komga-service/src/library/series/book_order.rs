//! The order `SeriesLifecycle.sortBooks` numbers books in: natural-comparator 1.1
//! `CaseInsensitiveSimpleNaturalComparator` over the trimmed, accent-stripped name with whitespace
//! runs collapsed.

use icu_normalizer::DecomposingNormalizerBorrowed;

/// The sort key: Kotlin `trim()`, commons-lang3 3.21 `stripAccents`, then Java `\s+` → `" "`.
pub(super) fn sort_key(name: &str) -> String {
    let trimmed = name.trim_matches(is_kotlin_whitespace);
    let decomposed = DecomposingNormalizerBorrowed::new_nfkd().normalize(trimmed);
    let mut key = String::with_capacity(decomposed.len());
    let mut in_whitespace = false;
    for character in decomposed.chars().map(remaining_accent) {
        if ('\u{0300}'..='\u{036f}').contains(&character) {
            continue;
        }
        let java_whitespace = matches!(character, ' ' | '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r');
        if !(java_whitespace && in_whitespace) {
            key.push(if java_whitespace { ' ' } else { character });
        }
        in_whitespace = java_whitespace;
    }
    key
}

/// Kotlin `Char.isWhitespace()`: Java `isWhitespace` or `isSpaceChar`.
fn is_kotlin_whitespace(character: char) -> bool {
    (character.is_whitespace() && character != '\u{85}')
        || ('\u{1c}'..='\u{1f}').contains(&character)
}

/// commons-lang3 `convertRemainingAccentCharacters`: letters with strokes or bars that NFKD keeps.
fn remaining_accent(character: char) -> char {
    match character {
        '\u{0141}' => 'L',
        '\u{0142}' => 'l',
        '\u{0110}' => 'D',
        '\u{0111}' => 'd',
        '\u{0197}' | '\u{1d7b}' | '\u{1da7}' => 'I',
        '\u{0268}' | '\u{1da4}' => 'i',
        '\u{0244}' | '\u{1d7e}' => 'U',
        '\u{0289}' | '\u{1db6}' => 'u',
        '\u{0166}' => 'T',
        '\u{0167}' => 't',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::sort_key;
    use crate::support::natural_order::compare;

    #[test]
    fn sorts_like_the_kotlin_book_order() {
        let names = [
            "Vol 10",
            "vol 2",
            "Vol 02",
            "Vol 2",
            "vol 1.5",
            "\u{c9}nigme 3",
            "enigme 10",
            "\u{141} 1",
            "\u{fb01}le 1",
            "  spaced\t\tname  ",
            "\u{130}stanbul",
            "istanbul",
            "a99999999999999999999",
            "a18446744073709551617",
            "a1",
            "\u{1f600} 1",
            "\u{1f600} 0",
            "\u{ff56}\u{ff4f}\u{ff4c} 3",
            "x\u{a0}y",
            "Book",
            "book",
            "BOOK",
            "\u{1c}lead",
            "\u{85}nel",
            "a\u{2003}\u{2003}b",
            "Chapter 001",
            "Chapter 1",
            "chapter 01",
        ];
        let mut sorted = names.to_vec();
        sorted.sort_by(|left, right| compare(&sort_key(left), &sort_key(right)));
        // Printed by JDK 21 with natural-comparator 1.1 and commons-lang3 3.21.0 for the same input.
        let expected = [
            ("a18446744073709551617", "a18446744073709551617"),
            ("a1", "a1"),
            ("a99999999999999999999", "a99999999999999999999"),
            ("a\u{2003}\u{2003}b", "a b"),
            ("Book", "Book"),
            ("book", "book"),
            ("BOOK", "BOOK"),
            ("Chapter 001", "Chapter 001"),
            ("Chapter 1", "Chapter 1"),
            ("chapter 01", "chapter 01"),
            ("\u{c9}nigme 3", "Enigme 3"),
            ("enigme 10", "enigme 10"),
            ("\u{fb01}le 1", "file 1"),
            ("\u{130}stanbul", "Istanbul"),
            ("istanbul", "istanbul"),
            ("\u{141} 1", "L 1"),
            ("\u{1c}lead", "lead"),
            ("  spaced\t\tname  ", "spaced name"),
            ("vol 1.5", "vol 1.5"),
            ("vol 2", "vol 2"),
            ("Vol 02", "Vol 02"),
            ("Vol 2", "Vol 2"),
            ("\u{ff56}\u{ff4f}\u{ff4c} 3", "vol 3"),
            ("Vol 10", "Vol 10"),
            ("x\u{a0}y", "x y"),
            ("\u{85}nel", "\u{85}nel"),
            ("\u{1f600} 0", "\u{1f600} 0"),
            ("\u{1f600} 1", "\u{1f600} 1"),
        ];
        let actual: Vec<(&str, String)> =
            sorted.iter().map(|name| (*name, sort_key(name))).collect();
        let expected: Vec<(&str, String)> = expected
            .iter()
            .map(|(name, key)| (*name, (*key).to_owned()))
            .collect();
        assert_eq!(actual, expected);
    }
}
