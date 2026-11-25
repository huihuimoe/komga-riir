//! Kotlin `ReadProgress`: how far a user has read a book.

use jiff::Timestamp;

use crate::{BookId, R2Locator, UserId};

#[derive(Debug, Clone, PartialEq)]
pub struct ReadProgress {
    pub book_id: BookId,
    pub user_id: UserId,
    /// 1-based; the page count once completed.
    pub page: i32,
    pub completed: bool,
    pub read_date: Timestamp,
    /// Empty unless a reading device reported the progress.
    pub device_id: String,
    pub device_name: String,
    pub locator: Option<R2Locator>,
}

/// Kotlin `R2Progression`: a Readium progression reported by a reader.
#[derive(Debug, Clone, PartialEq)]
pub struct R2Progression {
    pub modified: Timestamp,
    pub device: R2Device,
    pub locator: R2Locator,
}

/// Kotlin `R2Device`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct R2Device {
    pub id: String,
    pub name: String,
}
