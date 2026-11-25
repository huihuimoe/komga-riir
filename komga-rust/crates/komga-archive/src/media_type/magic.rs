//! Tika `Magic`, its clauses, and `MagicDetector` matching over a detection header.

use std::cell::OnceCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use fancy_regex::Regex;

use super::java_regex;

/// Compiled regexes by masked pattern; types that repeat a pattern share one regex and its
/// match caches.
pub(super) type Regexes = HashMap<Vec<u8>, Arc<Regex>>;

/// One `<magic>` alternative of a type; the registry keeps them in Tika `Magic` order.
pub(super) struct Magic {
    pub(super) type_index: usize,
    priority: i32,
    pub(super) clause: Clause,
    /// Tika `Magic.toString`, the last sort key.
    text: String,
}

impl Magic {
    pub(super) fn new(type_index: usize, priority: i32, clause: Clause) -> Self {
        let text = format!("[{priority}/{clause}]");
        Self {
            type_index,
            priority,
            clause,
            text,
        }
    }

    /// Tika `Magic.compareTo` given the type names: higher priority, then longer patterns,
    /// then descending type and text.
    pub(super) fn compare(&self, other: &Self, type_name: &str, other_type_name: &str) -> Ordering {
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| other.clause.size().cmp(&self.clause.size()))
            .then_with(|| other_type_name.cmp(type_name))
            .then_with(|| other.text.cmp(&self.text))
    }
}

/// The detection header. Matches read zeros past its end, as Tika's zero-filled buffers do.
pub(super) struct Header<'a> {
    bytes: &'a [u8],
    /// ISO-8859-1 text of the first `REGEX_EXTENT` bytes, zero-padded, with each character's
    /// byte offset, built for the first regex match.
    text: OnceCell<(String, Vec<usize>)>,
}

impl<'a> Header<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            text: OnceCell::new(),
        }
    }

    fn byte(&self, index: usize) -> u8 {
        self.bytes.get(index).copied().unwrap_or(0)
    }

    /// The `length` characters from `start`.
    fn region(&self, start: usize, length: usize) -> &str {
        let (text, offsets) = self.text.get_or_init(|| {
            let text: String = (0..REGEX_EXTENT)
                .map(|index| char::from(self.byte(index)))
                .collect();
            let mut offsets: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
            offsets.push(text.len());
            (text, offsets)
        });
        &text[offsets[start]..offsets[start + length]]
    }
}

/// Tika `Clause`: `<match>` elements nest as "this match and any of the nested ones".
pub(super) enum Clause {
    Match(Box<Match>),
    And(Vec<Clause>),
    Or(Vec<Clause>),
    MinShouldMatch { min: usize, clauses: Vec<Clause> },
}

impl Clause {
    pub(super) fn eval(&self, header: &Header<'_>) -> bool {
        match self {
            Self::Match(matcher) => matcher.eval(header),
            Self::And(clauses) => clauses.iter().all(|clause| clause.eval(header)),
            Self::Or(clauses) => clauses.iter().any(|clause| clause.eval(header)),
            Self::MinShouldMatch { min, clauses } => {
                clauses
                    .iter()
                    .filter(|clause| clause.eval(header))
                    .take(*min)
                    .count()
                    == *min
            }
        }
    }

    fn size(&self) -> usize {
        match self {
            Self::Match(matcher) => matcher.pattern.len(),
            Self::And(clauses) => clauses.iter().map(Self::size).sum(),
            Self::Or(clauses) | Self::MinShouldMatch { clauses, .. } => {
                clauses.iter().map(Self::size).max().unwrap_or(0)
            }
        }
    }
}

/// Java `toString`, which orders magics that tie on everything else.
impl fmt::Display for Clause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |f: &mut fmt::Formatter<'_>, clauses: &[Clause]| {
            let items: Vec<String> = clauses.iter().map(ToString::to_string).collect();
            write!(f, "[{}]", items.join(", "))
        };
        match self {
            Self::Match(matcher) => f.write_str(&matcher.text),
            Self::And(clauses) => {
                f.write_str("and")?;
                list(f, clauses)
            }
            Self::Or(clauses) => {
                f.write_str("or")?;
                list(f, clauses)
            }
            Self::MinShouldMatch { min, clauses } => {
                write!(f, "minShouldMatch (min: {min}) ")?;
                list(f, clauses)
            }
        }
    }
}

/// The attributes of a `<match>` element, as written in the registry.
pub(super) struct MatchSpec<'a> {
    pub(super) type_name: &'a str,
    pub(super) kind: &'a str,
    pub(super) offset: Option<&'a str>,
    pub(super) value: &'a str,
    pub(super) mask: Option<&'a str>,
}

/// Tika `MagicDetector`: a pattern under a mask anywhere in an offset range.
pub(super) struct Match {
    /// Masked pattern bytes, padded to the mask length.
    pattern: Vec<u8>,
    mask: Vec<u8>,
    begin: usize,
    end: usize,
    ignore_case: bool,
    regex: Option<Arc<Regex>>,
    /// Tika `MagicMatch.toString`.
    text: String,
}

/// Bytes a regex match looks at from each start offset.
const REGEX_WINDOW: usize = 8 * 1024;
/// Bytes from the header start that any regex window can reach.
const REGEX_EXTENT: usize = 2 * REGEX_WINDOW;

impl Match {
    /// Tika `MagicDetector.parse`.
    pub(super) fn parse(spec: &MatchSpec<'_>, regexes: &mut Regexes) -> Self {
        let (begin, end) = match spec.offset {
            None => (0, 0),
            Some(offset) => match offset.split_once(':') {
                Some((begin, end)) => (parse_offset(begin), parse_offset(end)),
                None => (parse_offset(offset), parse_offset(offset)),
            },
        };
        let pattern = decode_value(spec.value, spec.kind);
        let mask = spec.mask.map(|mask| decode_value(mask, spec.kind));
        let length = pattern.len().max(mask.as_ref().map_or(0, Vec::len));
        let mask: Vec<u8> = (0..length)
            .map(|index| {
                mask.as_ref()
                    .and_then(|mask| mask.get(index))
                    .copied()
                    .unwrap_or(0xff)
            })
            .collect();
        let pattern: Vec<u8> = (0..length)
            .map(|index| pattern.get(index).map_or(0, |byte| byte & mask[index]))
            .collect();
        let regex = (spec.kind == "regex").then(|| {
            Arc::clone(
                regexes
                    .entry(pattern.clone())
                    .or_insert_with(|| Arc::new(java_regex::compile(&pattern))),
            )
        });
        assert!(
            regex.is_none() || end + REGEX_WINDOW <= REGEX_EXTENT,
            "Regex offsets of {} reach past the padded header",
            spec.type_name
        );
        Self {
            text: format!(
                "{} {} {} {} {}",
                spec.type_name,
                spec.kind,
                spec.offset.unwrap_or("null"),
                spec.value,
                spec.mask.unwrap_or("null")
            ),
            pattern,
            mask,
            begin,
            end,
            ignore_case: spec.kind == "stringignorecase",
            regex,
        }
    }

    /// Tika `MagicDetector.detect`.
    fn eval(&self, header: &Header<'_>) -> bool {
        let Some(available) = header.bytes.len().checked_sub(self.begin) else {
            return false;
        };
        let range = self.end - self.begin;
        if let Some(regex) = &self.regex {
            // Java `lookingAt` on each region; a backtracking limit is the only error, which
            // Java does not have, so it counts as no match.
            return (0..=range).any(|start| {
                let region = header.region(self.begin + start, REGEX_WINDOW);
                regex.is_match(region).unwrap_or(false)
            });
        }
        let length = self.pattern.len();
        if available < length {
            return false;
        }
        (0..=range).any(|start| {
            (0..length).all(|index| {
                let masked = header.byte(self.begin + start + index) & self.mask[index];
                // Java `Character.toLowerCase` of the sign-extended byte: ASCII only.
                let masked = if self.ignore_case {
                    masked.to_ascii_lowercase()
                } else {
                    masked
                };
                masked == self.pattern[index]
            })
        })
    }
}

fn parse_offset(offset: &str) -> usize {
    offset
        .parse()
        .expect("Registry offsets are non-negative integers")
}

/// Tika `MagicDetector.decodeValue`.
fn decode_value(value: &str, kind: &str) -> Vec<u8> {
    let (digits, radix) = match value.strip_prefix("0x") {
        Some(hex) => (hex, 16),
        None => (value, 8),
    };
    let number = || i64::from_str_radix(digits, radix).expect("Registry numbers are valid");
    match kind {
        "string" | "regex" | "unicodeLE" | "unicodeBE" => decode_string(value, kind),
        "stringignorecase" => decode_string(&value.to_lowercase(), kind),
        "byte" => digits.as_bytes().to_vec(),
        "host16" | "little16" => number().to_le_bytes()[..2].to_vec(),
        "big16" => number().to_be_bytes()[6..].to_vec(),
        "host32" | "little32" => number().to_le_bytes()[..4].to_vec(),
        "big32" => number().to_be_bytes()[4..].to_vec(),
        _ => panic!("Unknown registry match type {kind}"),
    }
}

/// Tika `MagicDetector.decodeString`: `\\`, `\xHH`, `\r`, `\n` and up to three octal digits
/// are escapes; any other escaped character yields a zero byte and is kept.
fn decode_string(value: &str, kind: &str) -> Vec<u8> {
    if let Some(hex) = value.strip_prefix("0x") {
        return (0..hex.len() / 2)
            .map(|index| {
                u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                    .expect("Registry hex strings are valid")
            })
            .collect();
    }
    let chars: Vec<char> = value.chars().collect();
    let mut decoded: Vec<u16> = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '\\' {
            decoded.push(u16::try_from(u32::from(chars[index])).unwrap_or(0xfffd));
            index += 1;
            continue;
        }
        match chars[index + 1] {
            '\\' => {
                decoded.push(u16::from(b'\\'));
                index += 2;
            }
            'x' => {
                let hex: String = chars[index + 2..index + 4].iter().collect();
                decoded
                    .push(u16::from_str_radix(&hex, 16).expect("Registry hex escapes are valid"));
                index += 4;
            }
            'r' => {
                decoded.push(u16::from(b'\r'));
                index += 2;
            }
            'n' => {
                decoded.push(u16::from(b'\n'));
                index += 2;
            }
            _ => {
                let digits: String = chars[index + 1..]
                    .iter()
                    .take(3)
                    .take_while(|char| char.is_ascii_digit())
                    .collect();
                let octal = if digits.is_empty() {
                    0
                } else {
                    u16::from_str_radix(&digits, 8).expect("Registry octal escapes are valid")
                };
                // Java writes `Short.byteValue()` as a char, sign-extending bytes above 0x7f.
                let [byte, _] = octal.to_le_bytes();
                decoded.push(u16::from(byte) | if byte > 0x7f { 0xff00 } else { 0 });
                index += 1 + digits.len();
            }
        }
    }
    match kind {
        "unicodeLE" => decoded.iter().flat_map(|unit| unit.to_le_bytes()).collect(),
        "unicodeBE" => decoded.iter().flat_map(|unit| unit.to_be_bytes()).collect(),
        // Java `(byte) char`.
        _ => decoded.iter().map(|unit| unit.to_le_bytes()[0]).collect(),
    }
}
