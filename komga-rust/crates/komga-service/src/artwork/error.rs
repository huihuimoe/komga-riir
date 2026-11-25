use std::io;

use komga_archive::divina::EntryError;
use komga_archive::epub::EpubError;
use komga_archive::mobi::MobiError;
use komga_archive::pdf::PdfError;
use komga_db::DbError;
use komga_image::ImageError;
use komga_types::{BookId, FileUrl};
use thiserror::Error;
use tokio::task::JoinError;

use super::thumbnail_generator::GeneratorError;
use crate::BlockingError;
use crate::settings::SettingsError;
use crate::support::access_policy::Access;

#[derive(Debug, Error)]
pub enum ArtworkError {
    #[error("Not found")]
    NotFound,
    /// `ContentRestrictionChecker` denied the book or series.
    #[error("Content restricted")]
    Restricted,
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("Thumbnail worker failed: {0}")]
    Worker(#[from] JoinError),
    #[error(transparent)]
    Blocking(#[from] BlockingError),
    #[error("Book URL {0} is not a local file")]
    InvalidUrl(FileUrl),
    #[error("Book {0} has no media")]
    MissingMedia(BookId),
    /// Kotlin `NoThumbnailFoundException`.
    #[error("Book {0} has no cover")]
    NoCover(BookId),
    #[error("Cannot read the cover: {0}")]
    Archive(EntryError),
    #[error("Cannot read the EPUB cover: {0}")]
    Epub(EpubError),
    #[error("Cannot read the MOBI cover: {0}")]
    Mobi(MobiError),
    #[error("Cannot render the cover: {0}")]
    Pdf(PdfError),
    #[error("Cannot make the thumbnail: {0}")]
    Image(ImageError),
    #[error("Cannot read local artwork: {0}")]
    Io(#[from] io::Error),
    /// The thumbnail belongs to another book or series than the one requested.
    #[error("Thumbnail of another owner")]
    OtherOwner,
    #[error("Only uploaded thumbnails can be deleted")]
    NotUploaded,
    /// An upload that `ContentDetector.isImage` rejects.
    #[error("Not an image")]
    NotImage,
    /// Oneshot series take their book's posters.
    #[error("Series is a oneshot")]
    Oneshot,
}

pub(super) fn generator_error(id: &BookId, error: GeneratorError) -> ArtworkError {
    match error {
        GeneratorError::NoCover => ArtworkError::NoCover(id.clone()),
        GeneratorError::Archive(error) => ArtworkError::Archive(error),
        GeneratorError::Epub(error) => ArtworkError::Epub(error),
        GeneratorError::Mobi(error) => ArtworkError::Mobi(error),
        GeneratorError::Pdf(error) => ArtworkError::Pdf(error),
        GeneratorError::Image(error) => ArtworkError::Image(error),
    }
}

/// The answer to a content restriction check by ID.
pub(super) fn check(access: Access) -> Result<(), ArtworkError> {
    match access {
        Access::Allowed => Ok(()),
        Access::Denied => Err(ArtworkError::Restricted),
        Access::Unknown => Err(ArtworkError::NotFound),
    }
}
