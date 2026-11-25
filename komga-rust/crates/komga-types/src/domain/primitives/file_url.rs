//! Java `Path.toUri().toURL().toString()`: the form Kotlin stores in `SERIES.URL`, `BOOK.URL`,
//! and the other URL columns. Scans match existing rows by this exact string.

use std::fmt;
use std::path::{Path, PathBuf};

use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileUrl(String);

impl FileUrl {
    /// A regular file at the absolute `path`.
    #[must_use]
    pub fn file(path: &Path) -> Self {
        Self(format!("file:{}", encode(path)))
    }

    /// A directory at the absolute `path`; Java appends `/` when the path is a directory.
    #[must_use]
    pub fn directory(path: &Path) -> Self {
        let mut encoded = encode(path);
        if !encoded.ends_with('/') {
            encoded.push('/');
        }
        Self(format!("file:{encoded}"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Kotlin `URL.toURI().toPath()`; `None` for a URL that is not a local file URL.
    #[must_use]
    pub fn to_path(&self) -> Option<PathBuf> {
        Url::parse(&self.0).ok()?.to_file_path().ok()
    }
}

impl From<String> for FileUrl {
    /// A stored URL, kept verbatim so that lookups compare the exact stored string.
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for FileUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

crate::sql::sql_text!(FileUrl);

/// `UnixUriUtils.toUri`: path bytes outside RFC 2396 `pchar`, `;`, and `/` become uppercase `%XX`.
#[cfg(unix)]
fn encode(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut encoded = String::new();
    for &byte in path.as_os_str().as_bytes() {
        push_byte(&mut encoded, byte);
    }
    encoded
}

/// `WindowsUriSupport.toUri` through the multi-argument `URI` constructor: `\` becomes `/`, a
/// leading `/` precedes the drive, and non-ASCII characters other than spaces and controls stay
/// unescaped. Inferred from the JDK source; not verified on Windows.
#[cfg(not(unix))]
fn encode(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::from("/");
    for character in path.chars() {
        if character.is_ascii() {
            push_byte(&mut encoded, character as u8);
        } else if character.is_whitespace() || character.is_control() {
            let mut buffer = [0; 4];
            for &byte in character.encode_utf8(&mut buffer).as_bytes() {
                push_byte(&mut encoded, byte);
            }
        } else {
            encoded.push(character);
        }
    }
    encoded
}

fn push_byte(encoded: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let allowed = byte.is_ascii_alphanumeric() || b"-_.!~*'():@&=+$,;/".contains(&byte);
    if allowed {
        encoded.push(char::from(byte));
    } else {
        encoded.push('%');
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::Path;

    use super::FileUrl;

    #[test]
    fn encodes_like_java_path_to_uri() {
        // Expected values printed by JDK 21 `Path.toUri().toURL()` for the same paths.
        let directory = Path::new("/lib/My Series [2020] #1%é;@&=+$,~'()!*");
        assert_eq!(
            FileUrl::directory(directory).as_str(),
            "file:/lib/My%20Series%20%5B2020%5D%20%231%25%C3%A9;@&=+$,~'()!*/"
        );
        assert_eq!(
            FileUrl::file(&directory.join("Vol 01 ?.cbz")).as_str(),
            "file:/lib/My%20Series%20%5B2020%5D%20%231%25%C3%A9;@&=+$,~'()!*/Vol%2001%20%3F.cbz"
        );
        assert_eq!(FileUrl::directory(Path::new("/")).as_str(), "file:/");
    }

    #[test]
    fn decodes_the_path_back() {
        let path = Path::new("/lib/My Series [2020] #1%é;@&=+$,~'()!*/Vol 01 ?.cbz");
        assert_eq!(FileUrl::file(path).to_path().as_deref(), Some(path));
        assert_eq!(FileUrl::from("http://host/a".to_owned()).to_path(), None);
    }
}
