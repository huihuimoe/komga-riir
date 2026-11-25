//! Java `Pattern` semantics for the registry's `regex` matches, run with `fancy-regex`.
//!
//! Tika matches them against ISO-8859-1 text. The registry's patterns use no construct beyond
//! those translated here; each one compiles when the registry loads.

use fancy_regex::Regex;

/// Compiles the decoded pattern bytes (read as UTF-8, like Java `new String(bytes, UTF_8)`)
/// anchored at the region start, as `lookingAt` is.
pub(super) fn compile(pattern: &[u8]) -> Regex {
    let java = String::from_utf8_lossy(pattern);
    let translated = translate(&java);
    Regex::new(&format!(r"\A(?:{translated})"))
        .unwrap_or_else(|error| panic!("Registry regex {java:?} must compile: {error}"))
}

/// Rewrites Java classes whose meaning differs: `.` (without `(?s)`) also stops at `\r` and
/// U+0085, and `\s`, `\d`, `\w`, `\b` are ASCII-only (JDK 19+ for `\b`).
///
/// `.` and bracket classes are also narrowed to the ISO-8859-1 characters the text can hold,
/// which matches the same input: Unicode-wide classes compile to UTF-8 automata that cost
/// megabytes under large repetitions such as `.{100,2000}`.
fn translate(java: &str) -> String {
    const WORD: &str = "A-Za-z0-9_";
    const LATIN_1: &str = "\\x00-\\xFF";
    let dot_all = java.starts_with("(?s)");
    let mut translated = String::with_capacity(java.len() * 2);
    let mut in_class = false;
    let mut chars = java.chars().peekable();
    while let Some(char) = chars.next() {
        match char {
            '\\' => {
                let escaped = chars.next().expect("A Java escape names a character");
                let class = match escaped {
                    's' => Some("\\t\\n\\x0B\\x0C\\r "),
                    'd' => Some("0-9"),
                    'w' => Some(WORD),
                    _ => None,
                };
                match (class, escaped) {
                    (Some(class), _) if in_class => translated.push_str(class),
                    (Some(class), _) => translated.push_str(&format!("[{class}]")),
                    (None, 'b') if !in_class => translated.push_str(&format!(
                        "(?:(?<=[{WORD}])(?![{WORD}])|(?<![{WORD}])(?=[{WORD}]))"
                    )),
                    (None, 'u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        translated.push_str(&format!("\\x{{{hex}}}"));
                    }
                    (None, _) => {
                        translated.push('\\');
                        translated.push(escaped);
                    }
                }
            }
            '[' if !in_class => {
                in_class = true;
                translated.push_str("[[");
                // A leading `]` (after an optional `^`) is a literal in both dialects.
                if chars.peek() == Some(&'^') {
                    translated.push(chars.next().expect("Peeked"));
                }
                if chars.peek() == Some(&']') {
                    chars.next();
                    translated.push_str("\\]");
                }
            }
            ']' if in_class => {
                in_class = false;
                translated.push_str(&format!("]&&{LATIN_1}]"));
            }
            '.' if !in_class && dot_all => translated.push_str(&format!("[{LATIN_1}]")),
            '.' if !in_class => translated.push_str(&format!("[[^\\n\\r\\x{{85}}]&&{LATIN_1}]")),
            _ => translated.push(char),
        }
    }
    translated
}
