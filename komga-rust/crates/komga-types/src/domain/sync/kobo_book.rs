//! What a Kobo device is told about a book (Kotlin `KoboDtoRepository.findBookMetadataByIds`).

use jiff::civil::{Date, DateTime};

use crate::{BookId, R2Locator, SeriesId, ThumbnailId};

#[derive(Debug, Clone, PartialEq)]
pub struct KoboBook {
    pub id: BookId,
    pub title: String,
    pub number: String,
    pub number_sort: f32,
    pub isbn: String,
    pub summary: String,
    pub release_date: Option<Date>,
    /// `BOOK_METADATA.CREATED_DATE`, in UTC.
    pub metadata_created: DateTime,
    pub series_id: SeriesId,
    pub series_title: String,
    pub publisher: String,
    pub language: String,
    pub oneshot: bool,
    pub file_size: i64,
    pub is_kepub: bool,
    pub is_fixed_layout: bool,
    /// The selected thumbnail.
    pub cover: Option<ThumbnailId>,
    pub authors: Vec<String>,
}

/// A user's progress on a book as a Kobo device reads it back (Kotlin `ReadProgress.toDto`).
#[derive(Debug, Clone, PartialEq)]
pub struct KoboReadProgress {
    pub book_id: BookId,
    /// In UTC, as both dates below.
    pub created: DateTime,
    pub last_modified: DateTime,
    pub completed: bool,
    pub locator: Option<R2Locator>,
}
