//! Kotlin `CommonBookController.getBookEpubResource` and `BookAnalyzer.getFileContent` for
//! EPUB books.

use std::io;

use komga_archive::divina::EntryError;
use komga_archive::mobi::{MOBI_MEDIA_TYPE, MobiError};
use komga_archive::zip::{self, ZipError};
use komga_types::{KomgaUser, MediaStatus};
use tracing::warn;

use super::{BookSource, ReadingError, ReadingService, page_content};
use crate::support::blocking;

/// `FONT_EXTENSIONS`: fonts are requested anonymously by the reader's CSS.
const FONT_EXTENSIONS: [&str; 6] = ["otf", "woff", "woff2", "eot", "ttf", "svg"];

/// A member of an EPUB file and the media type found at analysis.
pub struct EpubResource {
    pub bytes: Vec<u8>,
    pub media_type: Option<String>,
}

/// Whether `name` has a font extension (`FilenameUtils.getExtension`, any case), which lets
/// anonymous requests read it from any book.
#[must_use]
pub fn is_font(name: &str) -> bool {
    let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file_name.rsplit_once('.').is_some_and(|(_, extension)| {
        FONT_EXTENSIONS
            .iter()
            .any(|font| font.eq_ignore_ascii_case(extension))
    })
}

impl ReadingService {
    /// The analyzed file `name` of the EPUB book. `restricted_to` is the user whose content
    /// restrictions apply, `None` for fonts, which skip them. Only names of the book's files are
    /// read, so a name cannot reach outside the book.
    pub async fn read_epub_resource(
        &self,
        restricted_to: Option<&KomgaUser>,
        source: BookSource,
        name: &str,
    ) -> Result<EpubResource, ReadingError> {
        if !page_content::is_epub(&source.media) {
            return Err(ReadingError::IncompatibleProfile(source.media.media_type));
        }
        if let Some(user) = restricted_to {
            self.check_access(user, &source.book).await?;
        }
        let file = source
            .media
            .files
            .into_iter()
            .find(|file| file.file_name == name)
            .ok_or(ReadingError::BookFileNotFound)?;
        if source.media.status != MediaStatus::Ready {
            warn!(book = %source.book.id, "Book media is not ready, cannot get files");
            return Err(ReadingError::FilesNotReady);
        }
        let is_mobi = source.media.media_type.as_deref() == Some(MOBI_MEDIA_TYPE);
        let book = source.book;
        let path = book
            .url
            .to_path()
            .ok_or_else(|| ReadingError::InvalidUrl(book.url.clone()))?;
        let bytes = if is_mobi {
            self.read_mobi_entry(&book, path, file.file_name).await?
        } else {
            read_epub_entry(&book, path, file.file_name).await?
        };
        Ok(EpubResource {
            bytes,
            media_type: file.media_type,
        })
    }

    async fn read_mobi_entry(
        &self,
        book: &komga_types::Book,
        path: std::path::PathBuf,
        entry: String,
    ) -> Result<Vec<u8>, ReadingError> {
        let cache = self.mobi_cache.clone();
        let book_id = book.id.clone();
        blocking::run(move || {
            let publication = cache.open(&book_id, &path)?;
            Ok::<_, MobiError>(publication.resource_bytes(&entry))
        })
        .await?
        .map_err(|error| match error {
            MobiError::Io(io) if io.kind() == io::ErrorKind::NotFound => ReadingError::FileNotFound,
            error => {
                warn!(book = %book.id, url = %book.url, %error, "Cannot read MOBI resource");
                ReadingError::Mobi(error)
            }
        })?
        .ok_or(ReadingError::BookFileNotFound)
    }
}

async fn read_epub_entry(
    book: &komga_types::Book,
    path: std::path::PathBuf,
    entry: String,
) -> Result<Vec<u8>, ReadingError> {
    blocking::run(move || zip::read_entry(&path, &entry))
        .await?
        .map_err(|error| match error {
            ZipError::EntryNotFound(_) => ReadingError::BookFileNotFound,
            error => {
                warn!(book = %book.id, url = %book.url, %error, "Cannot read EPUB resource");
                ReadingError::Archive(EntryError::Zip(error))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::is_font;

    #[test]
    fn fonts_are_known_by_the_extension_of_the_last_segment() {
        for (name, expected) in [
            ("fonts/a.ttf", true),
            ("A.WOFF2", true),
            ("images/logo.svg", true),
            ("dir.ttf/a", false),
            ("dir\\a.otf", true),
            ("ttf", false),
            ("a.css", false),
            ("", false),
        ] {
            assert_eq!(is_font(name), expected, "{name}");
        }
    }
}
