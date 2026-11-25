//! Kotlin `normalizeHref` and `URLDecoder.decode`: hrefs as the ZIP member names Komga looks up.

use std::fmt;

/// Java `URLDecoder.decode` rejects a `%` not followed by two hex digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedHref(pub String);

impl fmt::Display for MalformedHref {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Malformed escape in href {:?}", self.0)
    }
}

impl std::error::Error for MalformedHref {}

/// Java `URLDecoder.decode(href, UTF_8)`: `+` is a space and `%xx` runs are UTF-8 bytes, with
/// replacement characters for invalid sequences.
pub fn url_decode(href: &str) -> Result<String, MalformedHref> {
    let malformed = || MalformedHref(href.to_owned());
    let mut decoded = String::with_capacity(href.len());
    let mut bytes = Vec::new();
    let mut chars = href.chars().peekable();
    while let Some(char) = chars.next() {
        match char {
            '%' => {
                let mut hex = String::new();
                hex.extend(chars.next());
                hex.extend(chars.next());
                // Java parses the two characters with `Integer.parseInt`, which also takes `+`.
                let byte = (hex.chars().count() == 2)
                    .then(|| u8::from_str_radix(&hex, 16).ok())
                    .flatten()
                    .ok_or_else(malformed)?;
                bytes.push(byte);
                if chars.peek() != Some(&'%') {
                    decoded.push_str(&String::from_utf8_lossy(&bytes));
                    bytes.clear();
                }
            }
            '+' => decoded.push(' '),
            other => decoded.push(other),
        }
    }
    Ok(decoded)
}

/// Kotlin `normalizeHref`: the part before the last `#` resolved against `base_dir` and
/// normalized (only when there is a directory), then the non-blank anchor again.
pub(crate) fn normalize_href(base_dir: Option<&str>, href: &str) -> String {
    let (base, anchor) = href.rsplit_once('#').unwrap_or((href, ""));
    let path = match base_dir {
        Some(dir) => normalize(&resolve(dir, base)),
        None => java_path(base),
    };
    if anchor.trim().is_empty() {
        path
    } else {
        format!("{path}#{anchor}")
    }
}

/// Java `Path.getParent()` of a Unix path string, `None` for a single name.
pub(crate) fn parent(path: &str) -> Option<String> {
    let path = java_path(path);
    match path.rfind('/') {
        Some(0) if path.len() > 1 => Some("/".to_owned()),
        Some(0) | None => None,
        Some(index) => Some(path[..index].to_owned()),
    }
}

/// Java `Path.resolve`: an absolute or empty `other` replaces or keeps `dir`.
pub(crate) fn resolve(dir: &str, other: &str) -> String {
    let other = java_path(other);
    if other.starts_with('/') {
        other
    } else if other.is_empty() {
        java_path(dir)
    } else if dir.is_empty() {
        other
    } else {
        java_path(&format!("{dir}/{other}"))
    }
}

/// Java `Paths.get` on Unix: repeated and trailing slashes removed.
fn java_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let names: Vec<&str> = path.split('/').filter(|name| !name.is_empty()).collect();
    let joined = names.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

/// Java `Path.normalize`: `.` dropped, `name/..` folded, leading `..` of relative paths kept.
pub(crate) fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut names: Vec<&str> = Vec::new();
    for name in path.split('/').filter(|name| !name.is_empty()) {
        match name {
            "." => {}
            ".." if names.last().is_some_and(|last| *last != "..") => {
                names.pop();
            }
            ".." if absolute => {}
            name => names.push(name),
        }
    }
    let joined = names.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_like_url_decoder() {
        assert_eq!(
            url_decode("chapter%20027.xhtml").unwrap(),
            "chapter 027.xhtml"
        );
        assert_eq!(url_decode("a+b%3a%E2%80%99").unwrap(), "a b:\u{2019}");
        assert_eq!(url_decode("%FF").unwrap(), "\u{fffd}");
        assert!(
            url_decode("100%").is_err(),
            "Java rejects a truncated escape"
        );
        assert!(url_decode("%zz").is_err(), "Java rejects non-hex escapes");
    }

    #[test]
    fn normalizes_like_java_paths() {
        assert_eq!(normalize_href(Some("OEBPS"), "../img/a.jpg"), "img/a.jpg");
        assert_eq!(
            normalize_href(Some("OEBPS"), "./a.xhtml#x"),
            "OEBPS/a.xhtml#x"
        );
        assert_eq!(normalize_href(Some("a"), "../../b"), "../b");
        assert_eq!(normalize_href(Some("a"), "/b//c"), "/b/c");
        assert_eq!(
            normalize_href(Some("a"), "b#"),
            "a/b",
            "A blank anchor is dropped"
        );
        assert_eq!(normalize_href(Some("a"), ""), "a");
        // Without an OPF directory Kotlin does not normalize.
        assert_eq!(normalize_href(None, "./x/../a.xhtml"), "./x/../a.xhtml");
        assert_eq!(parent("OEBPS/content.opf").as_deref(), Some("OEBPS"));
        assert_eq!(parent("content.opf"), None);
        assert_eq!(parent("/content.opf").as_deref(), Some("/"));
    }
}
