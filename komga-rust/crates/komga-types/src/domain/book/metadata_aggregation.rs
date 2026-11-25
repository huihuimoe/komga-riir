//! Kotlin `BookMetadataAggregation`.

use jiff::civil::Date;

use super::metadata::{BookAuthor, NotableEntity};
use crate::ids::SeriesId;

/// What the metadata of the books of a series have in common, stored for the series.
#[derive(Debug, Clone, PartialEq)]
pub struct BookMetadataAggregation {
    pub series_id: SeriesId,
    pub authors: Vec<BookAuthor>,
    pub notable_entities: Vec<NotableEntity>,
    /// A set, in insertion order.
    pub tags: Vec<String>,
    pub release_date: Option<Date>,
    /// Empty when no book has a summary.
    pub summary: String,
    /// The number of the book `summary` comes from.
    pub summary_number: String,
}
