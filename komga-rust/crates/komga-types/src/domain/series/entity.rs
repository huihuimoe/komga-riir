//! Kotlin `Series`.

use jiff::civil::DateTime;

use crate::domain::FileUrl;
use crate::ids::{LibraryId, SeriesId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series {
    pub id: SeriesId,
    pub name: String,
    pub url: FileUrl,
    /// Server local time, like the Kotlin `LocalDateTime`.
    pub file_last_modified: DateTime,
    pub library_id: LibraryId,
    pub book_count: i32,
    pub deleted_date: Option<DateTime>,
    pub oneshot: bool,
}
