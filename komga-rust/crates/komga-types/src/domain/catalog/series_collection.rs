//! Kotlin `SeriesCollection`.

use jiff::civil::DateTime;

use crate::ids::{CollectionId, SeriesId};

/// A collection as one user sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesCollection {
    pub id: CollectionId,
    pub name: String,
    /// Whether the series are ordered manually rather than by title.
    pub ordered: bool,
    /// The visible series, ordered by their number in the collection.
    pub series_ids: Vec<SeriesId>,
    /// UTC.
    pub created_date: DateTime,
    /// UTC.
    pub last_modified_date: DateTime,
    /// Some series are hidden from the user.
    pub filtered: bool,
}

/// `SeriesCollectionDao.sorts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionSort {
    Name,
    CreatedDate,
    LastModifiedDate,
    /// Only applies with a full-text search: its most relevant hits first.
    Relevance,
}
