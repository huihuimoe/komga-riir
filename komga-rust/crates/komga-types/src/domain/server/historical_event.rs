//! Kotlin `HistoricalEvent`: a record of something done to a book or series.

use std::collections::BTreeMap;
use std::path::Path;

use jiff::civil::DateTime;

use crate::domain::{Book, BookPageNumbered};
use crate::{BookId, SeriesId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalEvent {
    pub id: String,
    pub kind: &'static str,
    pub book_id: Option<BookId>,
    pub series_id: Option<SeriesId>,
    pub properties: Vec<(&'static str, String)>,
}

/// A stored historical event, as listed to admins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalEventRecord {
    pub id: String,
    pub kind: String,
    pub timestamp: DateTime,
    pub book_id: Option<String>,
    pub series_id: Option<String>,
    pub properties: BTreeMap<String, String>,
}

/// The sort keys of historical events (`HistoricalEventDtoDao.sorts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoricalEventSort {
    Type,
    BookId,
    SeriesId,
    Timestamp,
}

impl HistoricalEvent {
    /// Kotlin `HistoricalEvent.DuplicatePageDeleted`; an unknown page size is left out.
    pub fn duplicate_page_deleted(book: &Book, page: &BookPageNumbered) -> Self {
        let path = book.url.to_path().expect("Books have file URLs");
        let mut properties = vec![
            ("name", path.to_string_lossy().into_owned()),
            ("page number", page.page_number.to_string()),
            ("page file name", page.file_name.clone()),
            ("page file hash", page.file_hash.clone()),
            ("page media type", page.media_type.clone()),
        ];
        if let Some(size) = page.file_size {
            properties.push(("page file size", size.to_string()));
        }
        Self {
            id: crate::ids::tsid::generate(),
            kind: "DuplicatePageDeleted",
            book_id: Some(book.id.clone()),
            series_id: Some(book.series_id.clone()),
            properties,
        }
    }

    /// Kotlin `HistoricalEvent.BookFileDeleted`.
    pub fn book_file_deleted(book: &Book, reason: &str) -> Self {
        let path = book.url.to_path().expect("Books have file URLs");
        Self {
            id: crate::ids::tsid::generate(),
            kind: "BookFileDeleted",
            book_id: Some(book.id.clone()),
            series_id: Some(book.series_id.clone()),
            properties: vec![
                ("reason", reason.to_owned()),
                ("name", path.to_string_lossy().into_owned()),
            ],
        }
    }

    /// Kotlin `HistoricalEvent.SeriesFolderDeleted`.
    pub fn series_folder_deleted(series_id: &SeriesId, path: &Path, reason: &str) -> Self {
        Self {
            id: crate::ids::tsid::generate(),
            kind: "SeriesFolderDeleted",
            book_id: None,
            series_id: Some(series_id.clone()),
            properties: vec![
                ("reason", reason.to_owned()),
                ("name", path.to_string_lossy().into_owned()),
            ],
        }
    }
}
