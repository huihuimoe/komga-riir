//! Pure-Rust MOBI6 reader and lossless runtime virtual-EPUB adapter (spec §16.4).
//!
//! Supports uncompressed, PalmDOC LZ77, and HUFF/CDIC (`17480`) MOBI6 books, including the MOBI6
//! section of Dual-MOBI files. Rejects DRM-protected books, standalone KF8 (`.azw3`), and unknown
//! compression/encoding schemes with [`MobiError::Unsupported`].

mod assembly;
mod binary;
mod decompress;
mod epub;
mod exth;
mod header;
mod html;
mod indx;

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use komga_types::EpubTocEntry;

use crate::epub::EpubMetadata;

/// MIME media type detected by `tika-mimetypes.xml` for `BOOKMOBI` files.
pub const MOBI_MEDIA_TYPE: &str = "application/x-mobipocket-ebook";
/// Largest `.mobi` file accepted into memory.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// Largest decompressed text body accepted (guards against compression bombs).
pub(super) const MAX_TEXT_BYTES: usize = 128 * 1024 * 1024;

/// Why a syntactically recognized MOBI file is intentionally unsupported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobiUnsupportedReason {
    Drm,
    Kf8,
    Compression(u16),
    Encoding(u32),
}

/// Errors returned while reading or normalizing a `.mobi` file.
#[derive(Debug, thiserror::Error)]
pub enum MobiError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("MOBI file of {0} bytes exceeds the {MAX_FILE_BYTES}-byte limit")]
    TooLarge(u64),
    #[error("MOBI resource limit exceeded: {0}")]
    LimitExceeded(&'static str),
    #[error("Unsupported MOBI variant: {0:?}")]
    Unsupported(MobiUnsupportedReason),
    #[error("Malformed MOBI file: {0}")]
    Invalid(String),
    #[error("MOBI contains no readable content")]
    NoReadableContent,
}

/// One normalized virtual XHTML chapter of a MOBI book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobiChapter {
    pub title: String,
    pub path: String,
    pub xhtml: String,
}

/// One embedded image record extracted from a MOBI book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobiImage {
    pub path: String,
    pub media_type: &'static str,
    pub bytes: Vec<u8>,
}

/// A parsed MOBI book normalized into a virtual EPUB publication.
#[derive(Debug, Clone)]
pub struct MobiPublication {
    pub(super) metadata: EpubMetadata,
    pub(super) unique_id: u32,
    pub(super) chapters: Vec<MobiChapter>,
    pub(super) images: Vec<MobiImage>,
    pub(super) cover_image_index: Option<usize>,
    pub(super) toc: Vec<EpubTocEntry>,
}

impl MobiPublication {
    /// Reads and normalizes the `.mobi` file at `path`.
    ///
    /// # Errors
    /// See [`read_file`] and [`Self::from_bytes`].
    pub fn open(path: &Path) -> Result<Self, MobiError> {
        let bytes = read_file(File::open(path)?)?;
        Self::from_bytes(&bytes)
    }

    /// Parses and normalizes an in-memory `.mobi` buffer.
    ///
    /// # Errors
    /// Returns [`MobiError::Unsupported`] for DRM/KF8/unknown compression/encoding,
    /// [`MobiError::LimitExceeded`] when decompression or image caps are exceeded, or
    /// [`MobiError::Invalid`] / [`MobiError::NoReadableContent`] for corrupt or empty input.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, MobiError> {
        let header = header::parse(bytes)?;
        let decoded = decompress::read_text(bytes, &header)?;
        let (images, recindex_map, cover_image_index) = assembly::extract_images(bytes, &header)?;
        let (body, body_offset) = html::extract_body(&decoded);
        let raw_chapters = html::split_chapters(body, body_offset);
        if raw_chapters.is_empty() && cover_image_index.is_none() {
            return Err(MobiError::NoReadableContent);
        }

        let indx_entries = indx::parse_toc(bytes, &header);
        let guide_entries = if indx_entries.is_empty() {
            html::parse_guide_toc(&decoded, &raw_chapters)
        } else {
            Vec::new()
        };

        let spans = assembly::build_chapter_spans(&raw_chapters);
        let chapters = assembly::build_chapters(
            &raw_chapters,
            &spans,
            &images,
            cover_image_index,
            &recindex_map,
            &indx_entries,
            &guide_entries,
        );
        let toc = assembly::build_toc(&indx_entries, &guide_entries, &spans, &chapters);

        Ok(Self {
            metadata: header.metadata,
            unique_id: header.unique_id,
            chapters,
            images,
            cover_image_index,
            toc,
        })
    }

    /// Parsed header and EXTH metadata in the same shape as `EpubPackage::metadata`.
    #[must_use]
    pub fn metadata(&self) -> &EpubMetadata {
        &self.metadata
    }

    /// Hierarchical or flat table of contents referencing virtual chapter paths.
    #[must_use]
    pub fn toc(&self) -> &[EpubTocEntry] {
        &self.toc
    }

    /// Approximate in-memory footprint in bytes, for bounded caching.
    #[must_use]
    pub fn memory_bytes(&self) -> u64 {
        let chapters: usize = self
            .chapters
            .iter()
            .map(|ch| ch.path.len() + ch.title.len() + ch.xhtml.len())
            .sum();
        let images: usize = self
            .images
            .iter()
            .map(|img| img.path.len() + img.bytes.len())
            .sum();
        (chapters + images).max(1) as u64
    }
}

/// Reads a `.mobi` file up to [`MAX_FILE_BYTES`].
///
/// # Errors
/// I/O error or [`MobiError::TooLarge`].
pub fn read_file(file: File) -> Result<Vec<u8>, MobiError> {
    let size = file.metadata()?.len();
    if size > MAX_FILE_BYTES {
        return Err(MobiError::TooLarge(size));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    let read = file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if read as u64 > MAX_FILE_BYTES {
        return Err(MobiError::TooLarge(read as u64));
    }
    Ok(bytes)
}

/// Reads only header and EXTH metadata from `path`, without decompressing the book text.
///
/// # Errors
/// See [`read_file`] and header parsing.
pub fn read_metadata(path: &Path) -> Result<EpubMetadata, MobiError> {
    let bytes = read_file(File::open(path)?)?;
    Ok(header::parse(&bytes)?.metadata)
}

/// Reads only the cover image bytes (EXTH `201` cover offset, falling back to `202` thumbnail
/// offset) without decompressing text or parsing HTML.
///
/// # Errors
/// See [`read_file`] and header parsing.
pub fn read_cover(path: &Path) -> Result<Option<Vec<u8>>, MobiError> {
    let bytes = read_file(File::open(path)?)?;
    let header = header::parse(&bytes)?;
    Ok(assembly::cover_bytes(&bytes, &header))
}

#[cfg(test)]
mod tests;
