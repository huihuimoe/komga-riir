//! Font families for the EPUB reader (Kotlin `FontsController`): the embedded families, plus one
//! directory per family under `komga.fonts.data-directory`, listed once at startup.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rust_embed::RustEmbed;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::{AsyncReadExt as _, Take};
use tracing::{error, info};

const SUPPORTED_EXTENSIONS: [&str; 4] = ["woff", "woff2", "ttf", "otf"];

#[derive(RustEmbed)]
#[folder = "../../../komga/src/main/resources/embeddedFonts/"]
struct EmbeddedFonts;

#[derive(Debug, Error)]
pub enum FontError {
    #[error("Font not found")]
    NotFound,
    #[error("Failed to read the font file: {0}")]
    Read(#[from] io::Error),
}

/// The fonts found at startup; files added later are not seen until a restart, as in Kotlin.
#[derive(Clone)]
pub struct FontService {
    families: Arc<Vec<Family>>,
}

struct Family {
    name: String,
    files: Vec<FontFile>,
}

struct FontFile {
    /// The name requests match.
    name: String,
    /// The last segment of the file URI, which Kotlin uses for the CSS and the media type.
    uri_name: String,
    source: Source,
}

enum Source {
    /// The path within [`EmbeddedFonts`].
    Embedded(Cow<'static, str>),
    Path(PathBuf),
}

/// A font file to send.
pub struct FontContent {
    /// `font/<extension>`.
    pub media_type: String,
    pub bytes: FontBytes,
}

pub enum FontBytes {
    /// Borrowed in release builds; debug builds read the source tree on each request.
    Embedded(Cow<'static, [u8]>),
    /// The file, cut at `length` so that the body never exceeds the announced length.
    File { content: Take<File>, length: u64 },
}

impl FontService {
    /// Lists the embedded fonts and the family directories of `data_directory`; a family directory
    /// named like an embedded family replaces it. Failing to list the directory is logged and
    /// leaves only the embedded fonts.
    pub fn load(data_directory: &Path) -> Self {
        let mut families = embedded();
        let discovered = discover(data_directory).unwrap_or_else(|error| {
            error!(directory = %data_directory.display(), %error, "Could not load additional fonts");
            Vec::new()
        });
        info!(fonts = %describe(&families), "Fonts embedded");
        info!(fonts = %describe(&discovered), "Fonts discovered");

        for family in discovered {
            match families.iter_mut().find(|known| known.name == family.name) {
                Some(known) => known.files = family.files,
                None => families.push(family),
            }
        }
        Self {
            families: Arc::new(families),
        }
    }

    pub fn families(&self) -> impl Iterator<Item = &str> {
        self.families.iter().map(|family| family.name.as_str())
    }

    /// # Errors
    /// [`FontError::NotFound`] for an unknown family or file name, [`FontError::Read`] when a
    /// listed file can no longer be read.
    pub async fn file(&self, family: &str, name: &str) -> Result<FontContent, FontError> {
        let file = self
            .family(family)
            .and_then(|family| family.files.iter().find(|file| file.name == name))
            .ok_or(FontError::NotFound)?;
        let media_type = format!("font/{}", extension(&file.uri_name).to_lowercase());
        let bytes = match &file.source {
            Source::Embedded(path) => {
                let embedded = EmbeddedFonts::get(path).ok_or(FontError::NotFound)?;
                FontBytes::Embedded(embedded.data)
            }
            Source::Path(path) => {
                let content = File::open(path).await?;
                let length = content.metadata().await?.len();
                FontBytes::File {
                    content: content.take(length),
                    length,
                }
            }
        };
        Ok(FontContent { media_type, bytes })
    }

    /// One `@font-face` block per style and weight, in the order the files first show them.
    pub fn css(&self, family: &str) -> Option<String> {
        let family = self.family(family)?;
        let mut faces: Vec<(Face, Vec<&FontFile>)> = Vec::new();
        for file in &family.files {
            let face = Face::of(&file.uri_name);
            match faces.iter_mut().find(|(known, _)| *known == face) {
                Some((_, files)) => files.push(file),
                None => faces.push((face, vec![file])),
            }
        }
        let blocks: Vec<String> = faces
            .iter()
            .map(|(face, files)| font_face(&family.name, *face, files))
            .collect();
        Some(blocks.join("\n"))
    }

    fn family(&self, name: &str) -> Option<&Family> {
        self.families.iter().find(|family| family.name == name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Face {
    style: &'static str,
    weight: &'static str,
}

impl Face {
    /// Kotlin `getFontCharacteristics`. URI names are ASCII, so ASCII case folding matches
    /// Kotlin's `ignoreCase`.
    fn of(uri_name: &str) -> Self {
        let lowercase = uri_name.to_ascii_lowercase();
        Self {
            style: if lowercase.contains("italic") {
                "italic"
            } else {
                "normal"
            },
            weight: if lowercase.contains("bold") {
                "bold"
            } else {
                "normal"
            },
        }
    }
}

/// Kotlin `buildFontFaceBlock`, after `trimIndent`.
fn font_face(family: &str, face: Face, files: &[&FontFile]) -> String {
    let family = escape_css_string(family);
    let sources: Vec<String> = files
        .iter()
        .map(|file| {
            let format = match extension(&file.uri_name).to_lowercase().as_str() {
                "ttf" => "truetype".to_owned(),
                "otf" => "opentype".to_owned(),
                other => other.to_owned(),
            };
            format!(
                "url('{}') format('{format}')",
                escape_css_string(&file.uri_name)
            )
        })
        .collect();
    let sources = sources.join(",");
    let Face { style, weight } = face;
    format!(
        r#"@font-face {{
    font-family: '{family}';
    src: {sources};
    font-weight: {weight};
    font-style: {style};
}}
"#
    )
}

/// Escapes characters that could break out of a single-quoted CSS string token.
fn escape_css_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str(r"\\"),
            '\'' => escaped.push_str(r"\'"),
            '\0'..='\x1f' | '\x7f' => {
                write!(escaped, "\\{:x} ", u32::from(ch)).expect("Writing to a String cannot fail");
            }
            other => escaped.push(other),
        }
    }
    escaped
}

/// The supported embedded files, grouped by their parent directory (Kotlin
/// `/embeddedFonts/**/*.*`), in path order.
fn embedded() -> Vec<Family> {
    let mut families: Vec<Family> = Vec::new();
    for path in EmbeddedFonts::iter() {
        let Some((directory, name)) = path.rsplit_once('/') else {
            continue;
        };
        if !is_supported(name) {
            continue;
        }
        let family = directory
            .rsplit_once('/')
            .map_or(directory, |(_, last)| last)
            .to_owned();
        let name = name.to_owned();
        let file = FontFile {
            uri_name: uri_segment(&name),
            name,
            source: Source::Embedded(path),
        };
        match families.iter_mut().find(|known| known.name == family) {
            Some(known) => known.files.push(file),
            None => families.push(Family {
                name: family,
                files: vec![file],
            }),
        }
    }
    families
}

fn is_supported(name: &str) -> bool {
    SUPPORTED_EXTENSIONS
        .iter()
        .any(|supported| supported.eq_ignore_ascii_case(extension(name)))
}

/// The family directories with their readable font files, sorted by name. Entries whose names
/// are not Unicode are skipped.
fn discover(directory: &Path) -> io::Result<Vec<Family>> {
    if !is_directory(directory) {
        return Ok(Vec::new());
    }
    let mut families = Vec::new();
    for (name, path) in sorted_entries(directory)? {
        if !is_directory(&path) {
            continue;
        }
        let mut files = Vec::new();
        for (file_name, file_path) in sorted_entries(&path)? {
            let regular = fs::metadata(&file_path).is_ok_and(|metadata| metadata.is_file());
            if is_supported(&file_name) && regular && fs::File::open(&file_path).is_ok() {
                files.push(FontFile {
                    uri_name: uri_segment(&file_name),
                    name: file_name,
                    source: Source::Path(file_path),
                });
            }
        }
        families.push(Family { name, files });
    }
    Ok(families)
}

fn sorted_entries(directory: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if let Ok(name) = entry.file_name().into_string() {
            entries.push((name, entry.path()));
        }
    }
    entries.sort();
    Ok(entries)
}

/// Java `Files.isDirectory`, which follows links and is false when the target is missing.
fn is_directory(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

/// Everything after the last dot, or nothing (Kotlin `Path.extension`, `getExtension`).
fn extension(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, extension)| extension)
}

/// A file name as the last segment of Java's `Path.toUri()` on Unix: bytes outside RFC 2396
/// `pchar` and `;` are percent-encoded in uppercase hex.
fn uri_segment(name: &str) -> String {
    let mut encoded = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'():@&=+$,;".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            write!(encoded, "%{byte:02X}").expect("Writing to a String cannot fail");
        }
    }
    encoded
}

fn describe(families: &[Family]) -> String {
    let described: Vec<String> = families
        .iter()
        .map(|family| {
            let files: Vec<&str> = family.files.iter().map(|file| file.name.as_str()).collect();
            format!("{}=[{}]", family.name, files.join(", "))
        })
        .collect();
    format!("{{{}}}", described.join(", "))
}

#[cfg(test)]
mod tests {
    use super::{FontService, uri_segment};

    #[test]
    fn embedded_css_groups_files_by_face() {
        let fonts = FontService::load(std::path::Path::new("/nonexistent"));
        let css = fonts.css("OpenDyslexic").expect("Embedded family");
        let expected = r#"@font-face {
    font-family: 'OpenDyslexic';
    src: url('OpenDyslexic-Bold-Italic.woff') format('woff'),url('OpenDyslexic-Bold-Italic.woff2') format('woff2');
    font-weight: bold;
    font-style: italic;
}
"#;
        assert!(css.starts_with(expected), "{css}");
        assert_eq!(css.matches("@font-face").count(), 4, "{css}");
        assert!(css.ends_with("font-style: normal;\n}\n"), "{css}");
    }

    #[test]
    fn uri_segments_encode_like_java() {
        assert_eq!(uri_segment("a-b_c.d!~*'()"), "a-b_c.d!~*'()");
        assert_eq!(uri_segment(":@&=+$,;"), ":@&=+$,;");
        assert_eq!(uri_segment("My Font#1%.ttf"), "My%20Font%231%25.ttf");
        assert_eq!(uri_segment("宋体.otf"), "%E5%AE%8B%E4%BD%93.otf");
    }

    #[test]
    fn css_escapes_quotes_backslashes_and_control_chars() {
        assert_eq!(
            super::escape_css_string("Bob's\\Font\nEvil"),
            r"Bob\'s\\Font\a Evil"
        );
    }
}
