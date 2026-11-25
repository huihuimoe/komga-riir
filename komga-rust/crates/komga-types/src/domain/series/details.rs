//! Kotlin `SeriesDto` with its `SeriesMetadataDto` and `BookMetadataAggregationDto`: the series
//! view the API shows to one user.

use jiff::civil::DateTime;

use crate::domain::{BookMetadataAggregation, Series, SeriesMetadata};

#[derive(Debug, Clone, PartialEq)]
pub struct SeriesDetails {
    pub series: Series,
    /// `SERIES.CREATED_DATE`, in UTC.
    pub created: DateTime,
    /// `SERIES.LAST_MODIFIED_DATE`, in UTC.
    pub last_modified: DateTime,
    /// Books the user of the view has completed.
    pub books_read_count: i32,
    /// Books the user of the view has started and not completed.
    pub books_in_progress_count: i32,
    pub metadata: SeriesMetadata,
    /// `SERIES_METADATA.CREATED_DATE`, in UTC.
    pub metadata_created: DateTime,
    /// `SERIES_METADATA.LAST_MODIFIED_DATE`, in UTC.
    pub metadata_last_modified: DateTime,
    pub books_metadata: BookMetadataAggregation,
    /// `BOOK_METADATA_AGGREGATION.CREATED_DATE`, in UTC.
    pub books_metadata_created: DateTime,
    /// `BOOK_METADATA_AGGREGATION.LAST_MODIFIED_DATE`, in UTC.
    pub books_metadata_last_modified: DateTime,
}

impl SeriesDetails {
    /// Kotlin's `bookCount - read - inProgress`.
    #[must_use]
    pub fn books_unread_count(&self) -> i32 {
        self.series.book_count - self.books_read_count - self.books_in_progress_count
    }
}
