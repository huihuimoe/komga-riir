//! Kotlin `ContentDetector`: Apache Tika 3.3.2 `MimeTypes` detection from content alone.
//!
//! Komga ships only `tika-core`, whose default detector is the magic registry of the embedded
//! `tika-mimetypes.xml` (copied unchanged; Apache License 2.0, see `TIKA_NOTICE`). Neither the
//! book file name nor the archive member name takes part, so file and member detection agree.

mod java_regex;
mod magic;
mod registry;
mod text;
mod xml_root;

use std::io::{self, Read};
use std::sync::LazyLock;

use registry::Registry;

/// Tika `MimeTypes.getMinLength`: detection only looks at this many leading bytes.
const HEADER_LENGTH: usize = 64 * 1024;

static REGISTRY: LazyLock<Registry> =
    LazyLock::new(|| Registry::parse(include_str!("tika-mimetypes.xml")));

/// Tika `Detector.detect` of the content `reader` starts with, as its media type text (for
/// example `image/jpeg` or `application/x-rar-compressed; version=4`).
pub fn detect(reader: impl Read) -> io::Result<&'static str> {
    let mut header = Vec::with_capacity(HEADER_LENGTH);
    reader.take(HEADER_LENGTH as u64).read_to_end(&mut header)?;
    Ok(REGISTRY.detect(&header))
}

/// The bytes Tika's `MimeTypes.detect` examines when Komga's `ZipExtractor` hands it an entry
/// stream wrapped in a Java `BufferedInputStream`: up to [`HEADER_LENGTH`] bytes, read the way
/// the marked buffer fills.
///
/// Each fill is one read of the decoded stream into the free part of a buffer that starts at
/// 8 KiB and doubles up to the mark limit. The read sizes matter for IMPLODING entries, whose
/// damaged data ends at a point that depends on them (`zip::ZipArchive::entry_reader`).
pub fn read_header(mut reader: impl Read) -> io::Result<Vec<u8>> {
    let mut header = vec![0; HEADER_LENGTH];
    let mut capacity = 8 * 1024;
    let mut filled = 0;
    while filled < HEADER_LENGTH {
        if filled == capacity {
            capacity = (capacity * 2).min(HEADER_LENGTH);
        }
        match reader.read(&mut header[filled..capacity])? {
            0 => break,
            count => filled += count,
        }
    }
    header.truncate(filled);
    Ok(header)
}

/// Kotlin `ContentDetector.mediaTypeToExtension` (Tika `forName(..).getExtension()`) for a
/// registered type or alias, such as [`detect`] returns: its first extension with the dot, or
/// `""` without one. `None` for other names; Tika would register a valid but unknown name with
/// no extension, which detected types never need.
#[must_use]
pub fn extension(media_type: &str) -> Option<&'static str> {
    REGISTRY.extension(media_type)
}

/// Kotlin `ContentDetector.isImage`.
#[must_use]
pub fn is_image(media_type: &str) -> bool {
    media_type.starts_with("image/")
}

#[cfg(test)]
mod tests;
