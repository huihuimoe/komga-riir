//! Rows shared by `THUMBNAIL_BOOK` and `THUMBNAIL_SERIES`, which have the same columns apart from
//! their owner.

mod book;
mod series;
mod set;

use komga_types::{Dimension, FileUrl, ThumbnailDetails, ThumbnailId, ThumbnailType};

use crate::DbError;

pub use book::ThumbnailBookDao;
pub use series::ThumbnailSeriesDao;
pub use set::{PosterSet, ThumbnailSetDao};

/// A thumbnail to insert, unselected.
pub struct NewThumbnail<'a> {
    pub id: &'a ThumbnailId,
    pub thumbnail: Option<&'a [u8]>,
    pub url: Option<&'a FileUrl>,
    pub kind: ThumbnailType,
    pub media_type: &'a str,
    pub file_size: i64,
    pub dimension: Dimension,
}

/// A thumbnail row without its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailEntry {
    pub id: ThumbnailId,
    pub kind: ThumbnailType,
    pub selected: bool,
    /// Whether the row has bytes or a URL; Kotlin `exists` without the file check.
    pub stored: bool,
}

/// The content of a thumbnail: its bytes, or for rows without them, the file they come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailContent {
    pub id: ThumbnailId,
    pub kind: ThumbnailType,
    pub thumbnail: Option<Vec<u8>>,
    pub url: Option<FileUrl>,
    pub media_type: String,
}

impl ThumbnailContent {
    /// Kotlin `ThumbnailBook.exists` and `ThumbnailSeries.exists` without the file check.
    #[must_use]
    pub fn is_stored(&self) -> bool {
        self.thumbnail.is_some() || self.url.is_some()
    }
}

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) struct ContentRow {
    id: ThumbnailId,
    #[sqlx(rename = "TYPE")]
    kind: ThumbnailType,
    thumbnail: Option<Vec<u8>>,
    url: Option<FileUrl>,
    media_type: String,
}

impl ContentRow {
    pub(super) fn into_content(self) -> Result<ThumbnailContent, DbError> {
        Ok(ThumbnailContent {
            id: self.id,
            kind: self.kind,
            thumbnail: self.thumbnail,
            url: self.url,
            media_type: self.media_type,
        })
    }
}

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) struct EntryRow {
    id: ThumbnailId,
    #[sqlx(rename = "TYPE")]
    kind: ThumbnailType,
    selected: bool,
    stored: bool,
}

impl EntryRow {
    pub(super) fn into_entry(self) -> Result<ThumbnailEntry, DbError> {
        Ok(ThumbnailEntry {
            id: self.id,
            kind: self.kind,
            selected: self.selected,
            stored: self.stored,
        })
    }
}

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) struct DetailsRow {
    id: ThumbnailId,
    #[sqlx(rename = "TYPE")]
    kind: ThumbnailType,
    selected: bool,
    media_type: String,
    file_size: i64,
    width: i64,
    height: i64,
}

impl DetailsRow {
    pub(super) fn into_details(self) -> Result<ThumbnailDetails, DbError> {
        let edge = |value: i64| {
            u32::try_from(value).map_err(|_| {
                DbError::UnsupportedSchema(format!("thumbnail dimension {value} out of range"))
            })
        };
        Ok(ThumbnailDetails {
            id: self.id,
            kind: self.kind,
            selected: self.selected,
            media_type: self.media_type,
            file_size: self.file_size,
            dimension: Dimension {
                width: edge(self.width)?,
                height: edge(self.height)?,
            },
        })
    }
}
