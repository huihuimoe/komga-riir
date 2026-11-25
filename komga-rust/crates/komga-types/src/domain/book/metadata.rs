//! Kotlin `BookMetadata` and its value types.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::ids::BookId;

/// Kotlin `BookMetadataPatchCapability`: which metadata fields a refresh may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BookMetadataPatchCapability {
    Title,
    Summary,
    Number,
    NumberSort,
    ReleaseDate,
    Authors,
    NotableEntities,
    Tags,
    Isbn,
    ReadLists,
    Thumbnails,
    Links,
}

impl BookMetadataPatchCapability {
    /// Kotlin `BookMetadataPatchCapability.entries`, which analyses and manual refreshes request.
    pub const ALL: [Self; 12] = [
        Self::Title,
        Self::Summary,
        Self::Number,
        Self::NumberSort,
        Self::ReleaseDate,
        Self::Authors,
        Self::NotableEntities,
        Self::Tags,
        Self::Isbn,
        Self::ReadLists,
        Self::Thumbnails,
        Self::Links,
    ];
}

/// Kotlin `BookMetadata`. Each field has a lock that keeps automatic refreshes from changing it.
#[derive(Debug, Clone, PartialEq)]
pub struct BookMetadata {
    pub book_id: BookId,
    pub title: String,
    pub title_lock: bool,
    pub summary: String,
    pub summary_lock: bool,
    pub number: String,
    pub number_lock: bool,
    pub number_sort: f32,
    pub number_sort_lock: bool,
    pub release_date: Option<Date>,
    pub release_date_lock: bool,
    pub authors: Vec<BookAuthor>,
    pub authors_lock: bool,
    pub notable_entities: Vec<NotableEntity>,
    pub notable_entities_lock: bool,
    /// A set, in insertion order.
    pub tags: Vec<String>,
    pub tags_lock: bool,
    /// Empty when unknown.
    pub isbn: String,
    pub isbn_lock: bool,
    pub links: Vec<WebLink>,
    pub links_lock: bool,
}

impl BookMetadata {
    /// The Kotlin constructor's normalization: title, summary, and number trimmed; tags
    /// lowercased, trimmed, non-blank, and distinct.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.title = self.title.trim().to_owned();
        self.summary = self.summary.trim().to_owned();
        self.number = self.number.trim().to_owned();
        self.tags = lower_not_blank(&self.tags);
        self
    }
}

/// Kotlin `lowerNotBlank().toSet()`: lowercased, trimmed, non-blank, and distinct, in order.
pub(crate) fn lower_not_blank(values: &[String]) -> Vec<String> {
    let mut distinct: Vec<String> = Vec::with_capacity(values.len());
    for value in values {
        let value = value.to_lowercase().trim().to_owned();
        if !value.is_empty() && !distinct.contains(&value) {
            distinct.push(value);
        }
    }
    distinct
}

/// Kotlin `Author`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookAuthor {
    pub name: String,
    pub role: String,
}

impl BookAuthor {
    /// Trims the name and the role, and lowercases the role.
    #[must_use]
    pub fn new(name: &str, role: &str) -> Self {
        Self {
            name: name.trim().to_owned(),
            role: role.trim().to_lowercase(),
        }
    }
}

/// Kotlin `NotableEntity`: a character, team, or location of a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotableEntity {
    pub kind: String,
    pub name: String,
}

impl NotableEntity {
    /// Trims the kind and the name, and lowercases the kind.
    #[must_use]
    pub fn new(kind: &str, name: &str) -> Self {
        Self {
            kind: kind.trim().to_lowercase(),
            name: name.trim().to_owned(),
        }
    }
}

/// Kotlin `WebLink`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebLink {
    pub label: String,
    pub url: String,
}
