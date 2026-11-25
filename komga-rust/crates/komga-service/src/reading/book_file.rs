//! Kotlin `CommonBookController.getBookFileInternal`: a book's file for download.

use std::error::Error;
use std::io::{BufWriter, Seek};
use std::path::PathBuf;

use komga_archive::epub::EpubPackage;
use komga_archive::mobi::MOBI_MEDIA_TYPE;
use komga_db::{BookDao, MediaDao};
use komga_types::{Book, BookId, KomgaUser, Media, MediaProfile, UserRole};
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, Take};
use tracing::warn;

use super::{ReadingError, ReadingService};
use crate::support::{access_policy, blocking};

const EPUB_MEDIA_TYPE: &str = "application/epub+zip";

/// An open book file and how to present it.
pub struct BookFile {
    /// The file, cut at `length` so that the body never exceeds the announced length.
    pub content: Take<File>,
    /// The size when the file was opened.
    pub length: u64,
    /// The file's own name, which Kotlin sends rather than the book name.
    pub file_name: String,
    pub media_type: Option<String>,
}

impl ReadingService {
    /// The file of a book a user with `FILE_DOWNLOAD` may see, whatever its media status.
    pub async fn book_file(&self, user: &KomgaUser, id: &BookId) -> Result<BookFile, ReadingError> {
        let (book, media) = self.downloadable(user, id).await?;
        open(&book, media.media_type).await
    }

    /// Kotlin `KoboController.getBookFile`: serves native EPUBs as is (or converts to KEPUB when
    /// `convert_kepub` is set), and dynamically packages `.mobi` books as `.epub` or `.kepub.epub`
    /// in full before streaming so a failed conversion is never a truncated download (spec §16.4).
    pub async fn kobo_book_file(
        &self,
        user: &KomgaUser,
        id: &BookId,
        convert_kepub: bool,
    ) -> Result<BookFile, ReadingError> {
        let (book, media) = self.downloadable(user, id).await?;
        // Opened first, so that a missing file is a 404 rather than a failed conversion.
        let original = open(&book, media.media_type).await?;
        let media_type = original.media_type.as_deref();
        let is_mobi = media_type == Some(MOBI_MEDIA_TYPE);
        if !convert_kepub {
            return if is_mobi {
                self.convert_to_epub(&book, true, false).await
            } else {
                Ok(original)
            };
        }
        if media.epub_is_kepub {
            return Ok(original);
        }
        if MediaProfile::from_media_type(media_type) != Some(MediaProfile::Epub) {
            let media_type = media_type.unwrap_or("Unknown media");
            return Err(ReadingError::Kepub(format!("{media_type} is not an EPUB")));
        }
        self.convert_to_epub(&book, is_mobi, true).await
    }

    async fn convert_to_epub(
        &self,
        book: &Book,
        is_mobi: bool,
        as_kepub: bool,
    ) -> Result<BookFile, ReadingError> {
        let path = book_path(book)?;
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let file_name = if as_kepub {
            format!("{stem}.kepub.epub")
        } else {
            format!("{stem}.epub")
        };
        let modified = book.file_last_modified;
        let cache = self.mobi_cache.clone();
        let book_id = book.id.clone();
        let converted = blocking::run(
            move || -> Result<std::fs::File, Box<dyn Error + Send + Sync>> {
                let mut out = BufWriter::new(tempfile::tempfile()?);
                if is_mobi {
                    let publication = cache.open(&book_id, &path)?;
                    if as_kepub {
                        publication.write_kepub(&mut out, modified)?;
                    } else {
                        publication.write_epub(&mut out, modified)?;
                    }
                } else {
                    EpubPackage::open(&path)?.write_kepub(&mut out, modified)?;
                }
                let mut out = out.into_inner()?;
                out.rewind()?;
                Ok(out)
            },
        )
        .await?
        .map_err(|error| {
            warn!(book = %book.id, %error, "EPUB/Kepub conversion failed");
            ReadingError::Kepub(error.to_string())
        })?;
        let file = File::from_std(converted);
        let length = file.metadata().await.map_err(ReadingError::Io)?.len();
        Ok(BookFile {
            content: file.take(length),
            length,
            file_name,
            media_type: Some(EPUB_MEDIA_TYPE.to_owned()),
        })
    }

    /// The book and its media, once the user is known to be allowed to download it.
    async fn downloadable(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<(Book, Media), ReadingError> {
        if !user.has_role(UserRole::FileDownload) {
            return Err(ReadingError::Forbidden);
        }
        let mut tx = self.db.begin().await?;
        let book = BookDao::find_by_id(&mut tx, id)
            .await?
            .ok_or(ReadingError::NotFound)?;
        if !access_policy::can_access_book(&mut tx, user, &book).await? {
            return Err(ReadingError::Restricted);
        }
        let media = MediaDao::find_by_id(&mut tx, id)
            .await?
            .ok_or_else(|| ReadingError::MissingMedia(id.clone()))?;
        tx.rollback().await?;
        Ok((book, media))
    }
}

fn book_path(book: &Book) -> Result<PathBuf, ReadingError> {
    book.url
        .to_path()
        .ok_or_else(|| ReadingError::InvalidUrl(book.url.clone()))
}

async fn open(book: &Book, media_type: Option<String>) -> Result<BookFile, ReadingError> {
    let path = book_path(book)?;
    let file = match File::open(&path).await {
        Ok(file) => file,
        // `FileSystemResource.exists`: a file whose existence cannot be checked is missing.
        Err(_) if !fs::try_exists(&path).await.unwrap_or(false) => {
            warn!(book = %book.id, url = %book.url, "File not found");
            return Err(ReadingError::FileNotFound);
        }
        Err(error) => return Err(ReadingError::Io(error)),
    };
    let length = file.metadata().await.map_err(ReadingError::Io)?.len();
    let file_name = path
        .file_name()
        .expect("Book URLs name a file")
        .to_string_lossy()
        .into_owned();
    Ok(BookFile {
        content: file.take(length),
        length,
        file_name,
        media_type,
    })
}
