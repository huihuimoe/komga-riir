//! Strongly typed identifiers for persisted Komga entities.

pub(crate) mod tsid;

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! tsid_newtype {
    ($(#[doc = $doc:literal] $name:ident),+ $(,)?) => {
        $(
            #[doc = $doc]
            #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
            #[serde(transparent)]
            pub struct $name(String);

            impl $name {
                /// Generate a Kotlin-compatible 13-character TSID.
                #[must_use]
                pub fn generate() -> Self {
                    Self(tsid::generate())
                }

                /// Borrow the identifier's string representation.
                #[must_use]
                pub fn as_str(&self) -> &str {
                    &self.0
                }
            }

            impl From<String> for $name {
                fn from(value: String) -> Self {
                    Self(value)
                }
            }

            impl From<&str> for $name {
                fn from(value: &str) -> Self {
                    Self(value.to_owned())
                }
            }

            impl AsRef<str> for $name {
                fn as_ref(&self) -> &str {
                    self.as_str()
                }
            }

            impl fmt::Display for $name {
                fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str(self.as_str())
                }
            }
        )+

        crate::sql::sql_text!($($name),+);
    };
}

tsid_newtype! {
    /// Identifier for a Komga user.
    UserId,
    /// Identifier for a user's API key.
    ApiKeyId,
    /// Identifier for a library.
    LibraryId,
    /// Identifier for a series.
    SeriesId,
    /// Identifier for a book.
    BookId,
    /// Identifier for a book or series thumbnail.
    ThumbnailId,
    /// Identifier for a read list.
    ReadListId,
    /// Identifier for a series collection.
    CollectionId,
    /// Identifier for a sync point.
    SyncPointId,
}
