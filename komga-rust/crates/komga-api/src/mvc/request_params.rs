//! Query-string binding with Spring MVC `@RequestParam` conversion rules.

use std::collections::BTreeSet;
use std::str::FromStr;

use axum::http::StatusCode;
use jiff::civil::Date;
use jiff::fmt::strtime;
use komga_types::AuthorMatch;

use crate::error::ApiError;

/// Decoded query parameters in request order.
pub(crate) struct RequestParams(Vec<(String, String)>);

/// A value Spring cannot convert to the parameter's type.
#[derive(Debug)]
pub(crate) struct TypeMismatch {
    pub(crate) name: &'static str,
    pub(crate) required_type: &'static str,
    /// The message of the `PropertyEditor` that rejected the value.
    pub(crate) cause: String,
}

impl ApiError {
    /// `MethodArgumentTypeMismatchException` message after the `PropertyEditor` fallback fails.
    pub(crate) fn type_mismatch(mismatch: &TypeMismatch, path: &str) -> Self {
        let TypeMismatch {
            name,
            required_type,
            cause,
        } = mismatch;
        Self::new(
            StatusCode::BAD_REQUEST,
            format!(
                "Method parameter '{name}': Failed to convert value of type 'java.lang.String' \
                 to required type '{required_type}'; {cause}"
            ),
            path,
        )
    }
}

impl RequestParams {
    pub(crate) fn parse(query: Option<&str>) -> Self {
        let pairs = form_urlencoded::parse(query.unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
        Self(pairs)
    }

    /// Adds a parameter after the query string's, as multipart form fields are.
    pub(crate) fn push(&mut self, name: String, value: String) {
        self.0.push((name, value));
    }

    /// Servlet `getParameter`: the first value.
    pub(crate) fn first(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Servlet `getParameterValues`: every value, in request order.
    pub(crate) fn all(&self, name: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .collect()
    }

    /// An optional list: values are split on commas and trimmed, and empty elements skipped.
    /// The error is the message of a 400.
    pub(crate) fn list<T: FromStr>(&self, name: &str) -> Result<Option<Vec<T>>, String> {
        let values = self.all(name);
        if values.is_empty() {
            return Ok(None);
        }
        let elements = values
            .into_iter()
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .filter(|element| !element.is_empty());
        let mut list = Vec::new();
        for element in elements {
            let parsed = element
                .parse()
                .map_err(|_| format!("Invalid value for parameter '{name}': {element}"))?;
            list.push(parsed);
        }
        Ok(Some(list))
    }

    /// An optional positive integer; blank means absent. The error is the message of a 400.
    pub(crate) fn positive_integer(&self, name: &str) -> Result<Option<u32>, String> {
        let Some(raw) = self
            .first(name)
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
        else {
            return Ok(None);
        };
        match raw.parse::<u32>() {
            Ok(value) if value > 0 => Ok(Some(value)),
            _ => Err(format!("Invalid value for parameter '{name}': {raw}")),
        }
    }

    /// `String`: Spring's conversion of repeated values joins them with commas.
    pub(crate) fn joined(&self, name: &str) -> Option<String> {
        let values: Vec<&str> = self
            .0
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .collect();
        (!values.is_empty()).then(|| values.join(","))
    }

    /// The full-text search of the `search` parameter, unless blank (`isNullOrBlank`).
    pub(crate) fn full_text_search(&self) -> Option<String> {
        self.joined("search")
            .filter(|search| !search.trim().is_empty())
    }

    /// `Set<String>`: one value is split on commas and trimmed; repeated values are kept as sent.
    pub(crate) fn string_set(&self, name: &str) -> BTreeSet<String> {
        let values: Vec<&str> = self
            .0
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
            .collect();
        match values.as_slice() {
            [""] => BTreeSet::new(),
            [single] => single
                .split(',')
                .map(|value| value.trim().to_owned())
                .collect(),
            _ => values.into_iter().map(str::to_owned).collect(),
        }
    }

    /// Optional `Boolean`: Spring's `StringToBooleanConverter`; blank means absent.
    pub(crate) fn boolean(&self, name: &'static str) -> Result<Option<bool>, TypeMismatch> {
        let Some(raw) = self.first(name) else {
            return Ok(None);
        };
        match raw.trim().to_lowercase().as_str() {
            "" => Ok(None),
            "true" | "on" | "yes" | "1" => Ok(Some(true)),
            "false" | "off" | "no" | "0" => Ok(Some(false)),
            _ => Err(TypeMismatch {
                name,
                required_type: "boolean",
                cause: format!("Invalid boolean value [{raw}]"),
            }),
        }
    }

    /// Optional `LocalDate` formatted as ISO `yyyy-MM-dd`; blank means absent.
    pub(crate) fn date(&self, name: &'static str) -> Result<Option<Date>, TypeMismatch> {
        let Some(raw) = self.first(name).filter(|raw| !raw.trim().is_empty()) else {
            return Ok(None);
        };
        strtime::parse("%Y-%m-%d", raw)
            .and_then(|parsed| parsed.to_date())
            .map(Some)
            .map_err(|error| TypeMismatch {
                name,
                required_type: "java.time.LocalDate",
                cause: format!("Invalid ISO date [{raw}]: {error}"),
            })
    }

    pub(crate) fn boolean_or(
        &self,
        name: &'static str,
        default: bool,
    ) -> Result<bool, TypeMismatch> {
        Ok(self.boolean(name)?.unwrap_or(default))
    }

    /// `AuthorsHandlerMethodArgumentResolver`: each `author` value is `name,role`, split at the
    /// last comma; values without a comma are skipped. Kotlin `Author` trims both and lowercases
    /// the role.
    pub(crate) fn authors(&self) -> Vec<AuthorMatch> {
        self.all("author")
            .into_iter()
            .filter_map(|value| value.rsplit_once(','))
            .map(|(name, role)| AuthorMatch {
                name: Some(name.trim().to_owned()),
                role: Some(role.trim().to_lowercase()),
            })
            .collect()
    }
}

/// An `int` path variable: Spring `NumberUtils.parseNumber` drops all Java whitespace, then
/// decodes `0x`, `0X` and `#` hex or parses decimal. Only ASCII digits are accepted, where Java
/// also accepts other Unicode decimal digits.
pub(crate) fn int_path_variable(name: &'static str, raw: &str) -> Result<i32, TypeMismatch> {
    let text: String = raw
        .chars()
        .filter(|character| !is_java_whitespace(*character))
        .collect();
    java_integer(&text).map_err(|cause| TypeMismatch {
        name,
        required_type: "int",
        cause,
    })
}

/// `Integer.decode` for hex numbers, `Integer.valueOf` otherwise; errors are Java's messages.
fn java_integer(text: &str) -> Result<i32, String> {
    let negative = text.starts_with('-');
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let hex = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
        .or_else(|| unsigned.strip_prefix('#'));
    let Some(digits) = hex else {
        return parse_int(text, 10);
    };
    if digits.starts_with(['-', '+']) {
        return Err("Sign character in wrong position".to_owned());
    }
    match parse_int(digits, 16) {
        Ok(value) if negative => Ok(-value),
        Ok(value) => Ok(value),
        // Only `-` and the digits reach `Integer.MIN_VALUE`.
        Err(_) if negative => parse_int(&format!("-{digits}"), 16),
        Err(error) => Err(error),
    }
}

/// `Integer.parseInt(text, radix)` over ASCII digits.
fn parse_int(text: &str, radix: u32) -> Result<i32, String> {
    let error = || match radix {
        10 => format!("For input string: \"{text}\""),
        _ => format!("For input string: \"{text}\" under radix {radix}"),
    };
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    if digits.is_empty() {
        return Err(error());
    }
    let mut value: i64 = 0;
    for character in digits.chars() {
        let digit = character.to_digit(radix).ok_or_else(error)?;
        value = value * i64::from(radix) + i64::from(digit);
        if value > 1 << 31 {
            return Err(error());
        }
    }
    i32::try_from(if negative { -value } else { value }).map_err(|_| error())
}

/// Java `Character.isWhitespace`: Unicode space separators other than no-break spaces, and
/// the ASCII controls `\t`..`\r` and `\u{1c}`..`\u{1f}`.
fn is_java_whitespace(character: char) -> bool {
    matches!(character, '\u{1c}'..='\u{1f}')
        || (character.is_whitespace()
            && !matches!(character, '\u{85}' | '\u{a0}' | '\u{2007}' | '\u{202f}'))
}

#[cfg(test)]
mod tests {
    use super::{RequestParams, int_path_variable};

    #[test]
    fn lists_split_trim_and_skip_empty_elements() {
        let list = |query: &str| RequestParams::parse(Some(query)).list::<i32>("n");
        assert_eq!(list("m=1"), Ok(None));
        assert_eq!(list("n="), Ok(Some(vec![])));
        assert_eq!(list("n=1,%202,&n=3&n="), Ok(Some(vec![1, 2, 3])));
        assert_eq!(
            list("n=1,x"),
            Err("Invalid value for parameter 'n': x".to_owned())
        );
    }

    #[test]
    fn positive_integers_reject_zero_and_text() {
        let size = |query: &str| RequestParams::parse(Some(query)).positive_integer("n");
        assert_eq!(size("n=%20"), Ok(None));
        assert_eq!(size("n=%2012"), Ok(Some(12)));
        for invalid in ["0", "-3", "x"] {
            assert_eq!(
                size(&format!("n={invalid}")),
                Err(format!("Invalid value for parameter 'n': {invalid}"))
            );
        }
    }

    #[test]
    fn int_path_variables_parse_like_spring() {
        // Spring Core 6.2 `NumberUtils.parseNumber(text, Integer.class)`.
        for (text, expected) in [
            ("abc", Err(r#"For input string: "abc""#)),
            ("1 2", Ok(12)),
            ("\t3\u{1f}", Ok(3)),
            ("+5", Ok(5)),
            ("-1", Ok(-1)),
            ("00012", Ok(12)),
            ("2147483647", Ok(i32::MAX)),
            ("2147483648", Err(r#"For input string: "2147483648""#)),
            ("0x10", Ok(16)),
            ("0X1f", Ok(31)),
            ("#10", Ok(16)),
            ("-0x10", Ok(-16)),
            ("+0x10", Err(r#"For input string: "+0x10""#)),
            ("0xZZ", Err(r#"For input string: "ZZ" under radix 16"#)),
            ("-0xZZ", Err(r#"For input string: "-ZZ" under radix 16"#)),
            ("0x", Err(r#"For input string: "" under radix 16"#)),
            ("0x-5", Err("Sign character in wrong position")),
            (
                "0x80000000",
                Err(r#"For input string: "80000000" under radix 16"#),
            ),
            ("-0x80000000", Ok(i32::MIN)),
            ("-", Err(r#"For input string: "-""#)),
            ("1.5", Err(r#"For input string: "1.5""#)),
            ("\u{a0}1", Err("For input string: \"\u{a0}1\"")),
        ] {
            let parsed = int_path_variable("n", text).map_err(|mismatch| mismatch.cause);
            assert_eq!(parsed, expected.map_err(str::to_owned), "{text:?}");
        }
    }
}
