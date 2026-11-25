//! Kotlin `ReadList`.

use jiff::civil::{Date, DateTime};

use crate::ids::{BookId, ReadListId, SeriesId};

/// A read list as one user sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadList {
    pub id: ReadListId,
    pub name: String,
    pub summary: String,
    /// Whether the books are ordered manually rather than by release date.
    pub ordered: bool,
    /// The visible books, ordered by their number in the list.
    pub book_ids: Vec<BookId>,
    /// UTC.
    pub created_date: DateTime,
    /// UTC.
    pub last_modified_date: DateTime,
    /// Some books are hidden from the user.
    pub filtered: bool,
}

/// `ReadListDao.sorts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadListSort {
    Name,
    CreatedDate,
    LastModifiedDate,
    /// Only applies with a full-text search: its most relevant hits first.
    Relevance,
}

/// Kotlin `ReadListRequest`: a read list to match against the books of the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadListRequest {
    pub name: String,
    pub books: Vec<ReadListRequestBook>,
}

/// Kotlin `ReadListRequestBook`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadListRequestBook {
    /// Series titles that may hold the book, without duplicates.
    pub series: Vec<String>,
    pub number: String,
}

/// The books of one series matching a [`ReadListRequestBook`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadListSeriesMatch {
    pub series_id: SeriesId,
    pub series_title: String,
    /// The aggregated release date of the series.
    pub release_date: Option<Date>,
    pub books: Vec<ReadListBookMatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadListBookMatch {
    pub book_id: BookId,
    pub number: String,
    pub title: String,
}
