//! Kotlin `BookDto` with its `MediaDto`, `BookMetadataDto`, and `ReadProgressDto`: the book view the
//! API shows to one user.

use jiff::civil::DateTime;

use crate::domain::{Book, BookMetadata, MediaStatus};

#[derive(Debug, Clone, PartialEq)]
pub struct BookDetails {
    pub book: Book,
    pub series_title: String,
    /// `BOOK.CREATED_DATE`, in UTC.
    pub created: DateTime,
    /// `BOOK.LAST_MODIFIED_DATE`, in UTC.
    pub last_modified: DateTime,
    pub media: MediaDetails,
    /// Authors, notable entities, and links in table order; tags in table order without
    /// duplicates.
    pub metadata: BookMetadata,
    /// `BOOK_METADATA.CREATED_DATE`, in UTC.
    pub metadata_created: DateTime,
    /// `BOOK_METADATA.LAST_MODIFIED_DATE`, in UTC.
    pub metadata_last_modified: DateTime,
    /// The progress of the user the view is for.
    pub read_progress: Option<ReadProgressDetails>,
}

/// Kotlin `MediaDto`: the media without its pages and files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDetails {
    pub status: MediaStatus,
    pub media_type: Option<String>,
    pub page_count: i32,
    pub comment: Option<String>,
    pub epub_divina_compatible: bool,
    pub epub_is_kepub: bool,
}

/// Kotlin `ReadProgressDto`. The dates are in UTC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadProgressDetails {
    pub page: i32,
    pub completed: bool,
    pub read_date: DateTime,
    pub created: DateTime,
    pub last_modified: DateTime,
    pub device_id: String,
    pub device_name: String,
}
