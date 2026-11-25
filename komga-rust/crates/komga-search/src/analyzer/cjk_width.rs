//! `CJKWidthFilter`.

use icu_normalizer::DecomposingNormalizerBorrowed;
use icu_normalizer::properties::CanonicalCompositionBorrowed;

/// Folds fullwidth ASCII variants to ASCII and halfwidth katakana to fullwidth, composing a
/// halfwidth (semi-)voiced sound mark with the katakana before it.
pub(super) fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\u{ff01}'..='\u{ff5e}' => folded.push(
                char::from_u32(u32::from(ch) - 0xfee0)
                    .expect("fullwidth ASCII variants map onto printable ASCII"),
            ),
            '\u{ff65}'..='\u{ff9f}' => {
                if let Some(voiced) = voice(folded.chars().next_back(), ch) {
                    folded.pop();
                    folded.push(voiced);
                } else {
                    // Each halfwidth katakana decomposes to its fullwidth form alone.
                    let mut buffer = [0; 4];
                    let halfwidth: &str = ch.encode_utf8(&mut buffer);
                    folded
                        .push_str(&DecomposingNormalizerBorrowed::new_nfkd().normalize(halfwidth));
                }
            }
            _ => folded.push(ch),
        }
    }
    folded
}

/// The katakana `previous` voiced by the halfwidth `mark`, when they compose.
fn voice(previous: Option<char>, mark: char) -> Option<char> {
    let combining = match mark {
        '\u{ff9e}' => '\u{3099}',
        '\u{ff9f}' => '\u{309a}',
        _ => return None,
    };
    let previous = previous.filter(|ch| ('\u{30a6}'..='\u{30fd}').contains(ch))?;
    CanonicalCompositionBorrowed::new().compose(previous, combining)
}
