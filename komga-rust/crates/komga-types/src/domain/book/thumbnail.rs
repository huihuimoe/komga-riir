//! Kotlin `ThumbnailBook.Type`, and the thumbnail fields clients list.

use std::str::FromStr;

use serde::Deserialize;

use crate::{Dimension, ThumbnailId};

/// A thumbnail without its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailDetails {
    pub id: ThumbnailId,
    pub kind: ThumbnailType,
    pub selected: bool,
    pub media_type: String,
    pub file_size: i64,
    pub dimension: Dimension,
}

/// Where a book thumbnail comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ThumbnailType {
    /// Made from the book's cover; a book has at most one.
    Generated,
    /// Copied from a local artwork file next to the book.
    Sidecar,
    UserUploaded,
}

impl ThumbnailType {
    /// The enum constant name, as persisted in `TYPE`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Generated => "GENERATED",
            Self::Sidecar => "SIDECAR",
            Self::UserUploaded => "USER_UPLOADED",
        }
    }
}

impl FromStr for ThumbnailType {
    type Err = ();

    /// Exact constant name, like Kotlin `ThumbnailBook.Type.valueOf`.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "GENERATED" => Ok(Self::Generated),
            "SIDECAR" => Ok(Self::Sidecar),
            "USER_UPLOADED" => Ok(Self::UserUploaded),
            _ => Err(()),
        }
    }
}

crate::sql::sql_enum!(ThumbnailType => name);
