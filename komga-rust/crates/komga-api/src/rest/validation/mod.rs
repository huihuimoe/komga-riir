//! Request-body checks equivalent to the Jakarta Bean Validation constraints on baseline DTOs.
//!
//! Semantics follow Hibernate Validator 8.0 as shipped with Spring Boot 3.5.

use std::borrow::Cow;

use komga_types::{is_context_path, is_isbn13, normalize_language_tag};
use url::Url;

use crate::error::Violation;

const NOT_BLANK_MESSAGE: &str = "must not be blank";
const EMAIL_MESSAGE: &str = "must be a well-formed email address";
const POSITIVE_OR_ZERO_MESSAGE: &str = "must be greater than or equal to 0";
const POSITIVE_MESSAGE: &str = "must be greater than 0";
const MAX_PORT_MESSAGE: &str = "must be less than or equal to 65535";
const CONTEXT_PATH_MESSAGE: &str = r#"must match "^/[\w-/]*[a-zA-Z0-9]$""#;
const NOT_NULL_MESSAGE: &str = "must not be null";
const NOT_EMPTY_MESSAGE: &str = "must not be empty";
const NULL_OR_NOT_EMPTY_MESSAGE: &str = "Must be null or not empty";
const UNIQUE_ELEMENTS_MESSAGE: &str = "must only contain unique elements";
const NULL_OR_NOT_BLANK_MESSAGE: &str = "Must be null or not blank";
const NULL_OR_BLANK_OR_ISBN_MESSAGE: &str = "Must be null or blank or valid ISBN-13";
const URL_MESSAGE: &str = "must be a valid URL";
const NULL_OR_BLANK_OR_BCP47_MESSAGE: &str = "Must be null or blank or valid BCP 47 language tag";
const CLIENT_SETTING_KEY_MESSAGE: &str =
    r#"must match "^[a-z](?:[a-z0-9_-]*[a-z0-9])*(?:\.[a-z0-9](?:[a-z0-9_-]*[a-z0-9])*)*$""#;

/// Java `String.trim()` strips every code point up to U+0020, not Unicode whitespace.
pub(crate) fn not_blank(
    field_name: impl Into<Cow<'static, str>>,
    value: &str,
    violations: &mut Vec<Violation>,
) {
    if value.trim_matches(|character| character <= ' ').is_empty() {
        violations.push(Violation::new(field_name, NOT_BLANK_MESSAGE));
    }
}

pub(crate) fn null_or_not_blank(
    field_name: &'static str,
    value: Option<&str>,
    violations: &mut Vec<Violation>,
) {
    if value.is_some_and(|value| value.trim_matches(|character| character <= ' ').is_empty()) {
        violations.push(Violation::new(field_name, NULL_OR_NOT_BLANK_MESSAGE));
    }
}

pub(crate) fn null_or_blank_or_isbn(
    field_name: &'static str,
    value: Option<&str>,
    violations: &mut Vec<Violation>,
) {
    if value.is_some_and(|value| !value.trim().is_empty() && !is_isbn13(value)) {
        violations.push(Violation::new(field_name, NULL_OR_BLANK_OR_ISBN_MESSAGE));
    }
}

pub(crate) fn null_or_blank_or_bcp47(
    field_name: &'static str,
    value: Option<&str>,
    violations: &mut Vec<Violation>,
) {
    if value
        .is_some_and(|value| !value.trim().is_empty() && normalize_language_tag(value).is_none())
    {
        violations.push(Violation::new(field_name, NULL_OR_BLANK_OR_BCP47_MESSAGE));
    }
}

/// Accepts an empty value; unlike the baseline, a missing URL is rejected.
pub(crate) fn url(
    field_name: impl Into<Cow<'static, str>>,
    value: Option<&str>,
    violations: &mut Vec<Violation>,
) {
    if !value.is_some_and(|value| value.is_empty() || Url::parse(value).is_ok()) {
        violations.push(Violation::new(field_name, URL_MESSAGE));
    }
}

pub(crate) fn not_null<T>(
    field_name: impl Into<Cow<'static, str>>,
    value: Option<&T>,
    violations: &mut Vec<Violation>,
) {
    if value.is_none() {
        violations.push(Violation::new(field_name, NOT_NULL_MESSAGE));
    }
}

pub(crate) fn not_empty<T>(
    field_name: &'static str,
    values: &[T],
    violations: &mut Vec<Violation>,
) {
    if values.is_empty() {
        violations.push(Violation::new(field_name, NOT_EMPTY_MESSAGE));
    }
}

pub(crate) fn null_or_not_empty<T>(
    field_name: &'static str,
    values: Option<&[T]>,
    violations: &mut Vec<Violation>,
) {
    if values.is_some_and(<[T]>::is_empty) {
        violations.push(Violation::new(field_name, NULL_OR_NOT_EMPTY_MESSAGE));
    }
}

pub(crate) fn unique_elements<T: Ord>(
    field_name: &'static str,
    values: Option<&[T]>,
    violations: &mut Vec<Violation>,
) {
    let Some(values) = values else {
        return;
    };
    let unique: std::collections::BTreeSet<&T> = values.iter().collect();
    if unique.len() != values.len() {
        violations.push(Violation::new(field_name, UNIQUE_ELEMENTS_MESSAGE));
    }
}

pub(crate) fn positive_or_zero(
    field_name: &'static str,
    value: i32,
    violations: &mut Vec<Violation>,
) {
    if value < 0 {
        violations.push(Violation::new(field_name, POSITIVE_OR_ZERO_MESSAGE));
    }
}

pub(crate) fn positive(field_name: &'static str, value: i64, violations: &mut Vec<Violation>) {
    if value <= 0 {
        violations.push(Violation::new(field_name, POSITIVE_MESSAGE));
    }
}

pub(crate) fn port(field_name: &'static str, value: i32, violations: &mut Vec<Violation>) {
    positive(field_name, value.into(), violations);
    if value > 65535 {
        violations.push(Violation::new(field_name, MAX_PORT_MESSAGE));
    }
}

pub(crate) fn context_path(field_name: &'static str, value: &str, violations: &mut Vec<Violation>) {
    if !is_context_path(value) {
        violations.push(Violation::new(field_name, CONTEXT_PATH_MESSAGE));
    }
}

/// `@Pattern` with the baseline `ClientSettingsController` key regex: lowercase dot-separated
/// segments of `[a-z0-9_-]` that start and end with a letter or digit, the first starting with
/// a letter.
pub(crate) fn client_setting_key(
    field_name: impl Into<Cow<'static, str>>,
    key: &str,
    violations: &mut Vec<Violation>,
) {
    let valid = key.split('.').enumerate().all(|(index, segment)| {
        let (Some(first), Some(last)) = (segment.chars().next(), segment.chars().last()) else {
            return false;
        };
        (first.is_ascii_lowercase() || (index > 0 && first.is_ascii_digit()))
            && (last.is_ascii_lowercase() || last.is_ascii_digit())
            && segment
                .chars()
                .all(|character| matches!(character, 'a'..='z' | '0'..='9' | '_' | '-'))
    });
    if !valid {
        violations.push(Violation::new(field_name, CLIENT_SETTING_KEY_MESSAGE));
    }
}

/// `@Email(regexp = ".+@.+\\..+")` as declared on the baseline user DTOs.
pub(crate) fn email(field_name: &'static str, value: &str, violations: &mut Vec<Violation>) {
    if !is_hibernate_email(value) || !matches_declared_email_pattern(value) {
        violations.push(Violation::new(field_name, EMAIL_MESSAGE));
    }
}

/// Full match of `.+@.+\..+`, where Java `.` excludes line terminators.
fn matches_declared_email_pattern(value: &str) -> bool {
    if value.chars().any(is_java_line_terminator) {
        return false;
    }
    value.char_indices().any(|(at, character)| {
        if character != '@' || at == 0 {
            return false;
        }
        let domain = &value[at + 1..];
        domain
            .char_indices()
            .any(|(dot, character)| character == '.' && dot > 0 && dot + 1 < domain.len())
    })
}

fn is_java_line_terminator(character: char) -> bool {
    matches!(character, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// `AbstractEmailValidator`: split at the last `@`, then check local part and domain.
///
/// Hibernate accepts an empty value here; the declared pattern rejects it instead.
fn is_hibernate_email(value: &str) -> bool {
    let Some((local_part, domain)) = value.rsplit_once('@') else {
        return false;
    };
    is_valid_local_part(local_part) && is_valid_email_domain(domain)
}

fn is_valid_local_part(local_part: &str) -> bool {
    // Java measures UTF-16 code units.
    if local_part.encode_utf16().count() > 64 {
        return false;
    }
    let mut rest = local_part;
    loop {
        let Some(after_word) = strip_local_word(rest) else {
            return false;
        };
        match after_word.strip_prefix('.') {
            Some(next) => rest = next,
            None => return after_word.is_empty(),
        }
    }
}

/// Consumes one `ATOM+` or quoted `"QATOM+"` word of the local part.
fn strip_local_word(input: &str) -> Option<&str> {
    let Some(mut rest) = input.strip_prefix('"') else {
        let end = input
            .find(|character| !is_local_atom(character))
            .unwrap_or(input.len());
        return (end > 0).then(|| &input[end..]);
    };
    let mut consumed = 0;
    loop {
        let mut characters = rest.chars();
        match characters.next()? {
            '"' if consumed > 0 => return Some(characters.as_str()),
            '\\' => match characters.next()? {
                '\\' | '"' => {}
                _ => return None,
            },
            character if is_quoted_local_atom(character) => {}
            _ => return None,
        }
        rest = characters.as_str();
        consumed += 1;
    }
}

fn is_local_atom(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || "!#$%&'*+/=?^_`{|}~-".contains(character)
        || character >= '\u{80}'
}

fn is_quoted_local_atom(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || "!#$%&'*.(),<>[]:; @+/=?^_`{|}~-".contains(character)
        || character >= '\u{80}'
}

/// `DomainNameUtil.isValidEmailDomainAddress`.
fn is_valid_email_domain(domain: &str) -> bool {
    if domain.ends_with('.') || !fits_idn_ascii_limits(domain) {
        return false;
    }
    if let Some(literal) = domain
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
    {
        return is_ipv4_literal(literal)
            || literal
                .strip_prefix("IPv6:")
                .is_some_and(|address| address.parse::<std::net::Ipv6Addr>().is_ok());
    }
    domain.split('.').all(is_domain_label)
}

/// `IDN.toASCII` rejects labels over 63 characters; the whole ASCII form is capped at 255.
///
/// Non-ASCII labels are measured after Punycode encoding without IDNA2003 nameprep, so labels
/// that nameprep would map, or reject as prohibited/unassigned, are not judged exactly like Java.
fn fits_idn_ascii_limits(domain: &str) -> bool {
    let mut total = 0;
    for (index, label) in domain.split('.').enumerate() {
        let length = if label.is_ascii() {
            label.len()
        } else {
            match idna::punycode::encode_str(label) {
                Some(encoded) => "xn--".len() + encoded.len(),
                None => return false,
            }
        };
        if length > 63 {
            return false;
        }
        total += length + usize::from(index > 0);
    }
    total <= 255
}

fn is_ipv4_literal(literal: &str) -> bool {
    let parts = literal.split('.').collect::<Vec<_>>();
    parts.len() == 4
        && parts.iter().all(|part| {
            (1..=3).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// `(?:C-*)*C+`: domain characters with inner dashes only.
fn is_domain_label(label: &str) -> bool {
    !label.is_empty()
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .chars()
            .all(|character| character == '-' || is_domain_character(character))
}

fn is_domain_character(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || "!#$%&'*+/=?^_`{|}~".contains(character)
        || character >= '\u{80}'
}

#[cfg(test)]
mod tests;
