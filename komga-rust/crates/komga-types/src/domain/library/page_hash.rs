//! Kotlin `PageHashKnown`, `PageHashUnknown`, and `PageHashMatch`: duplicate pages by hash.

use std::fmt;
use std::str::FromStr;

use jiff::civil::DateTime;
use serde::{Deserialize, Serialize};

use crate::{BookId, FileUrl};

/// Kotlin `PageHashKnown.Action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PageHashAction {
    DeleteAuto,
    DeleteManual,
    Ignore,
}

impl PageHashAction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeleteAuto => "DELETE_AUTO",
            Self::DeleteManual => "DELETE_MANUAL",
            Self::Ignore => "IGNORE",
        }
    }
}

impl fmt::Display for PageHashAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for PageHashAction {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "DELETE_AUTO" => Ok(Self::DeleteAuto),
            "DELETE_MANUAL" => Ok(Self::DeleteManual),
            "IGNORE" => Ok(Self::Ignore),
            other => Err(format!("Unknown page hash action {other}")),
        }
    }
}

/// A page hash marked with an action.
#[derive(Debug, Clone, PartialEq)]
pub struct PageHashKnown {
    pub hash: String,
    pub size: Option<i64>,
    pub action: PageHashAction,
    pub delete_count: i32,
    /// How many pages have the hash.
    pub match_count: i32,
    pub created_date: DateTime,
    pub last_modified_date: DateTime,
}

/// A hash several pages share that has no action yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageHashUnknown {
    pub hash: String,
    pub size: Option<i64>,
    pub match_count: i32,
}

/// A page with a given hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageHashMatch {
    pub book_id: BookId,
    pub url: FileUrl,
    /// From 1.
    pub page_number: i32,
    pub file_name: String,
    pub file_size: i64,
    pub media_type: String,
}

/// The sort keys of known hashes (`PageHashDao.sortsKnown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownPageHashSort {
    Hash,
    MatchCount,
    DeleteCount,
    /// `size * deleteCount`.
    DeleteSize,
    Size,
    Created,
    LastModified,
}

/// The sort keys of unknown hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownPageHashSort {
    Hash,
    Size,
    MatchCount,
    /// `size * matchCount`.
    TotalSize,
}

/// The sort keys of the pages with a hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageHashMatchSort {
    Size,
    Url,
    BookId,
    PageNumber,
}

crate::sql::sql_enum!(PageHashAction => as_str);
