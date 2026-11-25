//! Words as Lucene's `StandardTokenizer` finds them: UAX #29 word boundaries, with two additions
//! of its grammar (fullwidth digits are numeric, Southeast Asian runs stay whole) and its
//! selection of the segments that are words.

use std::iter::Peekable;
use std::ops::Range;

use icu_properties::props::{LineBreak, RegionalIndicator, Script, WordBreak};
use icu_properties::{CodePointMapData, CodePointSetData};
use unicode_segmentation::UnicodeSegmentation;

use super::emoji;

/// Lucene's default `maxTokenLength`; longer words are split.
const MAX_WORD_CHARS: usize = 255;

/// Ends a keycap emoji sequence such as `#️⃣`.
const COMBINING_KEYCAP: char = '\u{20e3}';

pub(super) struct Word {
    pub(super) offsets: Range<usize>,
    /// Han, Hiragana, Katakana and Hangul words, which `CJKBigramFilter` re-splits.
    pub(super) cjk: bool,
}

pub(super) fn words(text: &str) -> Vec<Word> {
    let mut words = Vec::new();
    for mut offsets in segments(text) {
        // UAX #29 attaches marks and modifiers to a preceding space, which Lucene leaves out.
        let segment = text[offsets.clone()].trim_start();
        offsets.start = offsets.end - segment.len();
        let Some(cjk) = word_kind(segment) else {
            continue;
        };
        let mut start = offsets.start;
        let chunk_ends = segment
            .char_indices()
            .skip(MAX_WORD_CHARS)
            .step_by(MAX_WORD_CHARS)
            .map(|(index, _)| offsets.start + index);
        for end in chunk_ends.chain([offsets.end]) {
            words.push(Word {
                offsets: start..end,
                cjk,
            });
            start = end;
        }
    }
    words
}

/// Word-break classes that matter to the joins UAX #29 misses.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Letter,
    Numeric,
    Katakana,
    ExtendNumLet,
    /// Joins digits (`MidNum`, `MidNumLet`, `Single_Quote`).
    MidNum,
    /// Attaches to the preceding character (`Extend`, `Format`, `ZWJ`).
    Attached,
    Other,
}

fn class(ch: char) -> Class {
    if is_fullwidth_digit(ch) {
        return Class::Numeric;
    }
    match CodePointMapData::<WordBreak>::new().get(ch) {
        WordBreak::ALetter | WordBreak::HebrewLetter => Class::Letter,
        WordBreak::Numeric => Class::Numeric,
        WordBreak::Katakana => Class::Katakana,
        WordBreak::ExtendNumLet => Class::ExtendNumLet,
        WordBreak::MidNum | WordBreak::MidNumLet | WordBreak::SingleQuote => Class::MidNum,
        WordBreak::Extend | WordBreak::Format | WordBreak::ZWJ => Class::Attached,
        _ => Class::Other,
    }
}

/// Lucene counts these as numeric; UAX #29 does not.
fn is_fullwidth_digit(ch: char) -> bool {
    ('\u{ff10}'..='\u{ff19}').contains(&ch)
}

fn is_southeast_asian(ch: char) -> bool {
    CodePointMapData::<LineBreak>::new().get(ch) == LineBreak::ComplexContext
}

fn base_chars(segment: &str) -> impl DoubleEndedIterator<Item = char> {
    segment.chars().filter(|ch| class(*ch) != Class::Attached)
}

/// UAX #29 segments, rejoined where Lucene's grammar joins them.
fn segments(text: &str) -> Vec<Range<usize>> {
    let mut joined: Vec<Range<usize>> = Vec::new();
    let mut pending = text
        .split_word_bound_indices()
        .map(|(start, segment)| start..start + segment.len())
        .peekable();
    while let Some(offsets) = pending.next() {
        if let Some(last) = joined.last_mut() {
            let (before, segment) = (&text[last.clone()], &text[offsets.clone()]);
            if joins(before, segment) {
                last.end = offsets.end;
                continue;
            }
            if let Some(after) = digits_across(before, segment, text, &mut pending) {
                last.end = after.end;
                continue;
            }
        }
        joined.push(offsets);
    }
    joined
}

/// Southeast Asian runs, and letters, digits and connectors next to a fullwidth digit.
fn joins(before: &str, segment: &str) -> bool {
    let all_southeast_asian =
        |text: &str| base_chars(text).next().is_some() && base_chars(text).all(is_southeast_asian);
    if all_southeast_asian(before) && all_southeast_asian(segment) {
        return true;
    }
    let (Some(tail), Some(head)) = (base_chars(before).next_back(), base_chars(segment).next())
    else {
        return false;
    };
    let word = |ch| {
        matches!(
            class(ch),
            Class::Letter | Class::Numeric | Class::ExtendNumLet
        )
    };
    (is_fullwidth_digit(tail) || is_fullwidth_digit(head)) && word(tail) && word(head)
}

/// The segment after `separator` when it joins `before` as digits around a fullwidth digit
/// (`１.５`), consuming it from `pending`.
fn digits_across(
    before: &str,
    separator: &str,
    text: &str,
    pending: &mut Peekable<impl Iterator<Item = Range<usize>>>,
) -> Option<Range<usize>> {
    let mut separator_chars = separator.chars();
    let (Some(mid), None) = (separator_chars.next(), separator_chars.next()) else {
        return None;
    };
    let tail = base_chars(before).next_back()?;
    let head = base_chars(&text[pending.peek()?.clone()]).next()?;
    let digits = class(tail) == Class::Numeric && class(head) == Class::Numeric;
    let fullwidth = is_fullwidth_digit(tail) || is_fullwidth_digit(head);
    if class(mid) == Class::MidNum && digits && fullwidth {
        pending.next()
    } else {
        None
    }
}

/// Whether the segment is a word, and whether it is bigrammed; `None` for spaces, punctuation
/// and other symbols.
fn word_kind(segment: &str) -> Option<bool> {
    let base: Vec<char> = base_chars(segment).collect();
    let all = |test: fn(char) -> bool| !base.is_empty() && base.iter().all(|ch| test(*ch));
    // Lucene's Han and Hiragana words are single characters; letters of those scripts such as
    // `々` are ordinary words.
    let cjk =
        all(|ch| matches!(script(ch), Script::Han | Script::Hiragana) && class(ch) == Class::Other)
            || all(|ch| class(ch) == Class::Katakana)
            || all(|ch| script(ch) == Script::Hangul && class(ch) == Class::Letter);
    if cjk {
        return Some(true);
    }
    // Southeast Asian marks and emoji modifiers are words even without a base character.
    let lexical = base
        .iter()
        .any(|ch| matches!(class(*ch), Class::Letter | Class::Numeric | Class::Katakana))
        || segment.chars().any(is_southeast_asian);
    // A lone regional indicator is no flag.
    let regional_indicators = segment
        .chars()
        .filter(|ch| CodePointSetData::new::<RegionalIndicator>().contains(*ch))
        .count();
    let emoji = regional_indicators >= 2
        || segment
            .chars()
            .any(|ch| emoji::is_pictographic(ch) || (ch == COMBINING_KEYCAP && !base.is_empty()));
    (lexical || emoji).then_some(false)
}

fn script(ch: char) -> Script {
    CodePointMapData::<Script>::new().get(ch)
}
