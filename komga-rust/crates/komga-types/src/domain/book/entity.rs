//! Kotlin `Book`.

use jiff::civil::DateTime;

use crate::domain::FileUrl;
use crate::ids::{BookId, LibraryId, SeriesId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub id: BookId,
    pub name: String,
    pub url: FileUrl,
    /// Server local time, like the Kotlin `LocalDateTime`.
    pub file_last_modified: DateTime,
    pub file_size: i64,
    pub file_hash: String,
    pub file_hash_koreader: String,
    pub number: i32,
    pub series_id: SeriesId,
    pub library_id: LibraryId,
    pub deleted_date: Option<DateTime>,
    pub oneshot: bool,
}

/// Kotlin `CopyMode`: how an imported file reaches its library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CopyMode {
    Move,
    Copy,
    /// A hard link, or a copy where the filesystem cannot link.
    Hardlink,
}

/// Kotlin `Task.ImportBook`: a file from outside every library to import into a series.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub source_file: String,
    pub series_id: SeriesId,
    pub copy_mode: CopyMode,
    pub destination_name: Option<String>,
    /// The book the file replaces.
    pub upgrade_book_id: Option<BookId>,
}
