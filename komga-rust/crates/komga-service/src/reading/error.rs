use std::io;

use komga_archive::divina::EntryError;
use komga_archive::mobi::MobiError;
use komga_archive::pdf::PdfError;
use komga_db::DbError;
use komga_image::ImageError;
use komga_types::{BookId, FileUrl, MediaStatus};
use thiserror::Error;

use crate::BlockingError;

#[derive(Debug, Error)]
pub enum ReadingError {
    #[error("Book not found")]
    NotFound,
    /// The user lacks the role the endpoint requires.
    #[error("Forbidden")]
    Forbidden,
    /// `ContentRestrictionChecker` denied the book.
    #[error("Content restricted")]
    Restricted,
    /// The media status that keeps a book's pages from being listed.
    #[error("Book media is {0}")]
    Unavailable(MediaStatus),
    /// Kotlin `MediaNotReadyException`.
    #[error("Book analysis failed")]
    MediaNotReady,
    /// Kotlin `MediaNotReadyException` of `getFileContent`, which no handler maps: a 500.
    #[error("Book media is not ready, cannot get files")]
    FilesNotReady,
    /// A profile-specific route asked for a book of another profile; the media type is `None`
    /// for unanalyzed books.
    #[error("Book media type '{}' not compatible with requested profile", .0.as_deref().unwrap_or("null"))]
    IncompatibleProfile(Option<String>),
    /// The book has no such file, or its file lost the member since the analysis.
    #[error("Book file not found")]
    BookFileNotFound,
    #[error("Page number does not exist")]
    PageOutOfBounds,
    #[error("Invalid conversion format: {0}")]
    InvalidConversion(String),
    /// Kotlin `ImageConversionException`; the book is named by ID, not by its path.
    #[error(
        "Convert page #{number} of book {book} from {from} to {to}: unsupported read format {from}"
    )]
    UnreadableFormat {
        number: i32,
        book: BookId,
        from: String,
        to: &'static str,
    },
    #[error("Cannot convert page: {0}")]
    Conversion(ImageError),
    /// Kotlin `MediaUnsupportedException` of `getPageContentRaw`.
    #[error("Extractor does not support raw extraction of pages")]
    RawUnsupported,
    /// Kotlin `MediaUnsupportedException` of `getPageContent`, which no handler maps: a 500.
    #[error("Epub profile does not support getting page content")]
    EpubPagesUnsupported,
    /// Kotlin `NoSuchFileException`: the book file is gone.
    #[error("File not found, it may have moved")]
    FileNotFound,
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error(transparent)]
    Blocking(#[from] BlockingError),
    #[error("Book URL {0} is not a local file")]
    InvalidUrl(FileUrl),
    #[error("Book {0} has no media")]
    MissingMedia(BookId),
    #[error("Cannot read book: {0}")]
    Archive(EntryError),
    #[error("Cannot read MOBI book: {0}")]
    Mobi(MobiError),
    #[error("Cannot read PDF page: {0}")]
    Pdf(PdfError),
    #[error("Cannot open book file: {0}")]
    Io(io::Error),
    #[error("Kepub conversion failed: {0}")]
    Kepub(String),
}
