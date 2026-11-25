//! `Accept` media ranges as Spring 6.2 `MediaType.parseMediaTypes` reads them, and the
//! specificity order of `MimeTypeUtils.sortBySpecificity`.

use axum::http::HeaderMap;
use axum::http::header::{ACCEPT, HeaderValue};

const MAX_RANGES: usize = 50;

#[derive(Debug, PartialEq)]
pub(crate) struct MediaRange {
    kind: String,
    subtype: String,
    parameters: Vec<(String, String)>,
    quality: f64,
}

#[derive(Debug, PartialEq)]
pub(crate) struct InvalidAccept(pub(crate) String);

impl MediaRange {
    fn new(kind: &str, subtype: &str) -> Self {
        Self {
            kind: kind.to_owned(),
            subtype: subtype.to_owned(),
            parameters: Vec::new(),
            quality: 1.0,
        }
    }

    fn is_wildcard_subtype(&self) -> bool {
        self.subtype == "*" || self.subtype.starts_with("*+")
    }

    fn subtype_suffix(&self) -> Option<&str> {
        self.subtype
            .rfind('+')
            .map(|index| &self.subtype[index + 1..])
    }

    fn is_compatible_with(&self, other: &Self) -> bool {
        if self.kind == "*" || other.kind == "*" {
            return true;
        }
        if self.kind != other.kind {
            return false;
        }
        if self.subtype == other.subtype {
            return true;
        }
        if !self.is_wildcard_subtype() && !other.is_wildcard_subtype() {
            return false;
        }
        let (suffix, other_suffix) = (self.subtype_suffix(), other.subtype_suffix());
        if self.subtype == "*" || other.subtype == "*" {
            true
        } else if let (true, Some(suffix)) = (self.is_wildcard_subtype(), suffix) {
            suffix == other.subtype || Some(suffix) == other_suffix
        } else if let (true, Some(other_suffix)) = (other.is_wildcard_subtype(), other_suffix) {
            self.subtype == other_suffix || Some(other_suffix) == suffix
        } else {
            false
        }
    }

    fn is_more_specific(&self, other: &Self) -> bool {
        if self.quality != other.quality {
            return self.quality > other.quality;
        }
        match (self.kind == "*", other.kind == "*") {
            (true, false) => return false,
            (false, true) => return true,
            _ => {}
        }
        match (self.is_wildcard_subtype(), other.is_wildcard_subtype()) {
            (true, false) => false,
            (false, true) => true,
            _ => {
                self.kind == other.kind
                    && self.subtype == other.subtype
                    && self.parameters.len() > other.parameters.len()
            }
        }
    }
}

pub(crate) fn parse(headers: &HeaderMap) -> Result<Vec<MediaRange>, InvalidAccept> {
    parse_values(headers.get_all(ACCEPT))
}

/// Parses `Accept` header values; rejects malformed ranges, non-ASCII, or >50 ranges.
pub(crate) fn parse_values<'a>(
    values: impl IntoIterator<Item = &'a HeaderValue>,
) -> Result<Vec<MediaRange>, InvalidAccept> {
    let mut ranges = Vec::new();
    for value in values {
        let value = value
            .to_str()
            .map_err(|_| InvalidAccept(String::from_utf8_lossy(value.as_bytes()).into_owned()))?;
        for token in tokenize(value) {
            if !java_trim(token).is_empty() {
                ranges.push(parse_media_type(token)?);
            }
        }
    }
    if ranges.len() > MAX_RANGES {
        return Err(InvalidAccept(format!("{} media types", ranges.len())));
    }
    Ok(ranges)
}

/// The type Spring's error controller writes its JSON body as: the most specific range that takes
/// `application/json` or a `+json` type, as `application/json` when it has wildcards; none when
/// no range takes JSON. Without ranges, `application/json`.
pub(crate) fn error_json_type(mut ranges: Vec<MediaRange>) -> Option<String> {
    let json = MediaRange::new("application", "json");
    let suffixed = MediaRange::new("application", "*+json");
    if ranges.is_empty() {
        return Some("application/json".to_owned());
    }
    sort_by_specificity(&mut ranges);
    let range = ranges
        .iter()
        .find(|range| range.is_compatible_with(&json) || range.is_compatible_with(&suffixed))?;
    Some(if range.kind == "*" || range.is_wildcard_subtype() {
        "application/json".to_owned()
    } else {
        format!("{}/{}", range.kind, range.subtype)
    })
}

/// Spring Security `HttpBasicConfigurer`'s matcher for its Basic challenge (`X-Requested-With`
/// aside): exactly `*/*` (no `Accept` included), or a REST type that HTML is not accepted with.
pub(crate) fn prefers_basic_challenge(headers: &HeaderMap) -> bool {
    const REST: [&str; 7] = [
        "application/atom+xml",
        "application/x-www-form-urlencoded",
        "application/json",
        "application/octet-stream",
        "application/xml",
        "multipart/form-data",
        "text/xml",
    ];
    let Ok(ranges) = parse(headers) else {
        return false;
    };
    let is_all = |range: &MediaRange| {
        range.kind == "*" && range.subtype == "*" && range.parameters.is_empty()
    };
    let html = MediaRange::new("text", "html");
    let rest = ranges.iter().filter(|range| !is_all(range)).any(|range| {
        REST.iter().any(|media_type| {
            range.is_compatible_with(&parse_media_type(media_type).expect("A media type"))
        })
    });
    ranges.is_empty()
        || ranges.iter().any(is_all)
        || (rest && !ranges.iter().any(|range| range.is_compatible_with(&html)))
}

/// `CommonBookController.getBookPageInternal`: whether the most specific range that accepts a
/// PDF or an image, by `sortBySpecificity`, accepts a PDF.
pub(crate) fn prefers_pdf(mut ranges: Vec<MediaRange>) -> bool {
    let pdf = MediaRange::new("application", "pdf");
    let image = MediaRange::new("image", "*");
    if !ranges.iter().any(|range| range.is_compatible_with(&pdf)) {
        return false;
    }
    ranges.retain(|range| range.is_compatible_with(&pdf) || range.is_compatible_with(&image));
    sort_by_specificity(&mut ranges);
    ranges[0].is_compatible_with(&pdf)
}

/// `MimeTypeUtils.sortBySpecificity`: `bubbleSort` with `isLessSpecific`, which is not a total
/// order.
fn sort_by_specificity(ranges: &mut [MediaRange]) {
    let len = ranges.len();
    for i in 0..len {
        for j in 1..len - i {
            if ranges[j].is_more_specific(&ranges[j - 1]) {
                ranges.swap(j - 1, j);
            }
        }
    }
}

/// Spring's `produces` negotiation: the first of `produces` compatible with the most specific
/// range any is compatible with; the first without ranges.
pub(crate) fn produced<'a>(mut ranges: Vec<MediaRange>, produces: &[&'a str]) -> Option<&'a str> {
    if ranges.is_empty() {
        return produces.first().copied();
    }
    sort_by_specificity(&mut ranges);
    ranges.iter().find_map(|range| {
        produces.iter().copied().find(|media_type| {
            range.is_compatible_with(&parse_media_type(media_type).expect("A media type"))
        })
    })
}

/// `ProducesRequestCondition`: whether a range accepts `produces`, a media type without wildcards,
/// and agrees on its parameters; no ranges accept all.
pub(crate) fn accepts(ranges: &[MediaRange], produces: &str) -> bool {
    let produces = parse_media_type(produces).expect("A media type");
    ranges.is_empty()
        || ranges.iter().any(|range| {
            produces.is_compatible_with(range)
                && produces.parameters.iter().all(|(name, value)| {
                    range
                        .parameters
                        .iter()
                        .find(|(other, _)| other == name)
                        .is_none_or(|(_, other)| other.eq_ignore_ascii_case(value))
                })
        })
}

/// Splits on commas outside quotes.
fn tokenize(value: &str) -> Vec<&str> {
    let bytes = value.as_bytes();
    let (mut tokens, mut start, mut in_quotes, mut i) = (Vec::new(), 0, false, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            b',' if !in_quotes => {
                tokens.push(&value[start..i]);
                start = i + 1;
            }
            b'\\' => i += 1,
            _ => {}
        }
        i += 1;
    }
    tokens.push(&value[start.min(value.len())..]);
    tokens
}

fn java_trim(value: &str) -> &str {
    value.trim_matches(|c: char| c <= ' ')
}

pub(crate) fn is_media_type(value: &str) -> bool {
    parse_media_type(value).is_ok()
}

/// Charset parameters are not checked against JVM charsets.
fn parse_media_type(value: &str) -> Result<MediaRange, InvalidAccept> {
    let invalid = || InvalidAccept(value.to_owned());
    let bytes = value.as_bytes();
    let semicolon = value.find(';');
    let full = java_trim(semicolon.map_or(value, |index| &value[..index]));
    let full = if full == "*" { "*/*" } else { full };
    let slash = full.find('/').ok_or_else(invalid)?;
    let (kind, subtype) = (&full[..slash], &full[slash + 1..]);
    if subtype.is_empty() || (kind == "*" && subtype != "*") || !is_token(kind) {
        return Err(invalid());
    }
    if !is_token(subtype) {
        return Err(invalid());
    }

    // Spring scans from -1 when there is no `;`, so the whole value is then one parameter.
    let mut parameters: Vec<(String, String)> = Vec::new();
    let mut start = semicolon.map_or(0, |index| index + 1);
    loop {
        let mut next = start;
        let mut quoted = false;
        while next < bytes.len() {
            match bytes[next] {
                b';' if !quoted => break,
                // Spring reads before the value there and fails.
                b'"' if next == 0 => return Err(invalid()),
                b'"' if bytes[next - 1] != b'\\' => quoted = !quoted,
                _ => {}
            }
            next += 1;
        }
        if let Some((name, parameter_value)) = java_trim(&value[start..next]).split_once('=') {
            let (name, parameter_value) = (java_trim(name), java_trim(parameter_value));
            if name.is_empty() || parameter_value.is_empty() || !is_token(name) {
                return Err(invalid());
            }
            if !is_quoted(parameter_value) && !is_token(parameter_value) {
                return Err(invalid());
            }
            // `LinkedCaseInsensitiveMap`: a later parameter of the same name replaces it.
            let name = name.to_ascii_lowercase();
            parameters.retain(|(existing, _)| *existing != name);
            parameters.push((name, parameter_value.to_owned()));
        }
        if next >= bytes.len() {
            break;
        }
        start = next + 1;
    }

    let quality = match parameters.iter().find(|(name, _)| name == "q") {
        None => 1.0,
        Some((_, quality)) => java_trim(unquote(quality))
            .parse::<f64>()
            .ok()
            .filter(|quality| (0.0..=1.0).contains(quality))
            .ok_or_else(invalid)?,
    };
    Ok(MediaRange {
        kind: kind.to_ascii_lowercase(),
        subtype: subtype.to_ascii_lowercase(),
        parameters,
        quality,
    })
}

/// RFC 2616 `token`, as `MimeType.TOKEN`.
fn is_token(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?={}".contains(&byte))
}

fn is_quoted(value: &str) -> bool {
    value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
}

fn unquote(value: &str) -> &str {
    if is_quoted(value) {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

#[cfg(test)]
mod tests;
