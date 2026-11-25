//! Kotlin `PageHashKnownDto`, `PageHashUnknownDto`, `PageHashMatchDto`, and
//! `PageHashCreationDto`.

use jiff::civil::DateTime;
use komga_types::{
    BookId, BookPageNumbered, PageHashAction, PageHashKnown, PageHashMatch, PageHashUnknown,
};
use serde::{Deserialize, Serialize};

use crate::error::Violation;
use crate::rest::validation;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageHashKnownDto {
    hash: String,
    size: Option<i64>,
    action: PageHashAction,
    delete_count: i32,
    match_count: i32,
    /// Jackson's `LocalDateTime` form, in UTC.
    created: DateTime,
    last_modified: DateTime,
}

impl From<PageHashKnown> for PageHashKnownDto {
    fn from(known: PageHashKnown) -> Self {
        Self {
            hash: known.hash,
            size: known.size,
            action: known.action,
            delete_count: known.delete_count,
            match_count: known.match_count,
            created: known.created_date,
            last_modified: known.last_modified_date,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageHashUnknownDto {
    hash: String,
    size: Option<i64>,
    match_count: i32,
}

impl From<PageHashUnknown> for PageHashUnknownDto {
    fn from(unknown: PageHashUnknown) -> Self {
        Self {
            hash: unknown.hash,
            size: unknown.size,
            match_count: unknown.match_count,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageHashMatchDto {
    book_id: BookId,
    /// The book's file path.
    url: String,
    page_number: i32,
    file_name: String,
    file_size: i64,
    media_type: String,
}

impl PageHashMatchDto {
    /// Kotlin `deleteSingleMatchByPageHash`: the book and its page with `hash`.
    pub(crate) fn into_page(self, hash: &str) -> (BookId, BookPageNumbered) {
        let page = BookPageNumbered {
            file_name: self.file_name,
            media_type: self.media_type,
            dimension: None,
            file_hash: hash.to_owned(),
            file_size: Some(self.file_size),
            page_number: self.page_number,
        };
        (self.book_id, page)
    }
}

impl From<PageHashMatch> for PageHashMatchDto {
    fn from(page: PageHashMatch) -> Self {
        let url = page
            .url
            .to_path()
            .expect("Books have file URLs")
            .to_string_lossy()
            .into_owned();
        Self {
            book_id: page.book_id,
            url,
            page_number: page.page_number,
            file_name: page.file_name,
            file_size: page.file_size,
            media_type: page.media_type,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PageHashCreationDto {
    pub hash: String,
    pub size: Option<i64>,
    pub action: PageHashAction,
}

impl PageHashCreationDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::not_blank("hash", &self.hash, &mut violations);
        violations
    }
}
