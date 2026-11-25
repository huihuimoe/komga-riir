//! Kotlin `BookAnalyzer.divinaExtractors`: the archive formats whose image members are pages,
//! keyed by the media types their extractors declare.

use std::io;
use std::path::Path;

use crate::rar::{self, RarError};
use crate::zip::{self, ZipError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DivinaFormat {
    Zip,
    Rar,
}

impl DivinaFormat {
    /// `ZipExtractor.mediaTypes` and `RarExtractor.mediaTypes`.
    #[must_use]
    pub fn from_media_type(media_type: &str) -> Option<Self> {
        match media_type {
            "application/zip" => Some(Self::Zip),
            "application/x-rar-compressed"
            | "application/x-rar-compressed; version=4"
            | "application/x-rar-compressed; version=5" => Some(Self::Rar),
            _ => None,
        }
    }

    /// Kotlin `getEntryStream`: the data of the member named `name`.
    ///
    /// # Errors
    /// The archive or the member cannot be read, or there is no such member.
    pub fn read_entry(self, path: &Path, name: &str) -> Result<Vec<u8>, EntryError> {
        Ok(match self {
            Self::Zip => zip::read_entry(path, name)?,
            Self::Rar => rar::read_entry(path, name)?,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EntryError {
    #[error(transparent)]
    Zip(#[from] ZipError),
    #[error(transparent)]
    Rar(#[from] RarError),
}

impl EntryError {
    /// Kotlin `NoSuchFileException`: the book file is gone.
    #[must_use]
    pub fn is_file_not_found(&self) -> bool {
        matches!(
            self,
            Self::Zip(ZipError::Io(cause)) | Self::Rar(RarError::Io(cause))
                if cause.kind() == io::ErrorKind::NotFound
        )
    }
}
