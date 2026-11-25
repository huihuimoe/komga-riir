//! Kotlin `LocalArtworkProvider.{getBookThumbnails,getSeriesThumbnails}` on the filesystem: the
//! image files next to a book or in a series directory, read whole into memory.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use komga_archive::media_type;
use komga_image::read_dimension;
use komga_types::{Dimension, FileUrl};
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tracing::{info, warn};

use crate::library::scanning::sidecar_names::{
    ARTWORK_EXTENSIONS, SERIES_ARTWORK_NAMES, base_name, is_book_sidecar,
};

/// The largest artwork file read whole, the same bound as a whole-read archive member.
const MAX_ARTWORK_BYTES: u64 = 256 * 1024 * 1024;

/// An artwork file with the properties Kotlin derives from it, taken from one read so that they
/// agree with the stored bytes.
pub(super) struct LocalArtwork {
    pub(super) url: FileUrl,
    pub(super) bytes: Vec<u8>,
    pub(super) media_type: &'static str,
    pub(super) dimension: Dimension,
}

/// The regular files next to `book_path` named after the book with an optional `-<digits>`
/// counter and an artwork extension, in name order.
pub(super) fn book_candidates(book_path: &Path) -> io::Result<Vec<PathBuf>> {
    let book_name = book_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(base_name)
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Book path has no file name"))?;
    let directory = book_path
        .parent()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "Book path has no parent"))?;
    candidates(directory, |file_name| is_book_sidecar(book_name, file_name))
}

/// The regular files in the series directory named like a series cover, in name order.
pub(super) fn series_candidates(directory: &Path) -> io::Result<Vec<PathBuf>> {
    candidates(directory, |file_name| {
        SERIES_ARTWORK_NAMES.contains(&base_name(file_name).to_lowercase().as_str())
    })
}

fn candidates(directory: &Path, matches: impl Fn(&str) -> bool) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        // Kotlin `Path.extension.lowercase()`, unlike the case-insensitive scan prefilter.
        let extension = file_name
            .rsplit_once('.')
            .map_or("", |(_, extension)| extension);
        if !matches(file_name) || !ARTWORK_EXTENSIONS.contains(&extension.to_lowercase().as_str()) {
            continue;
        }
        let path = entry.path();
        // `Files.isRegularFile` follows links and is false when the file cannot be inspected.
        if fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Reads the candidates that are images. Files gone since listing or over the size bound are
/// skipped; other read failures fail the whole refresh.
pub(super) async fn load(paths: Vec<PathBuf>) -> io::Result<Vec<LocalArtwork>> {
    let mut found = Vec::new();
    for path in paths {
        let bytes = match read_bounded(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) if error.kind() == ErrorKind::FileTooLarge => {
                warn!(path = %path.display(), "Local artwork is too large, skipping");
                continue;
            }
            Err(error) => return Err(error),
        };
        let media_type = media_type::detect(bytes.as_slice())?;
        if !media_type::is_image(media_type) {
            continue;
        }
        info!(path = %path.display(), "Found file");
        found.push(LocalArtwork {
            url: FileUrl::file(&path),
            dimension: read_dimension(bytes.as_slice()).unwrap_or(Dimension {
                width: 0,
                height: 0,
            }),
            bytes,
            media_type,
        });
    }
    Ok(found)
}

/// The whole file, or [`ErrorKind::FileTooLarge`] past [`MAX_ARTWORK_BYTES`].
pub(super) async fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)
        .await?
        .take(MAX_ARTWORK_BYTES + 1)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > MAX_ARTWORK_BYTES {
        return Err(io::Error::new(
            ErrorKind::FileTooLarge,
            format!("{} is over {MAX_ARTWORK_BYTES} bytes", path.display()),
        ));
    }
    Ok(bytes)
}
