//! Kotlin `SeriesMetadata`.

use std::str::FromStr;

use serde::Deserialize;

use crate::SeriesId;
use crate::domain::book::metadata::{WebLink, lower_not_blank};

/// Kotlin `SeriesMetadata.ReadingDirection`, stored by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadingDirection {
    LeftToRight,
    RightToLeft,
    Vertical,
    Webtoon,
}

impl FromStr for ReadingDirection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "LEFT_TO_RIGHT" => Ok(Self::LeftToRight),
            "RIGHT_TO_LEFT" => Ok(Self::RightToLeft),
            "VERTICAL" => Ok(Self::Vertical),
            "WEBTOON" => Ok(Self::Webtoon),
            other => Err(format!("Unknown reading direction {other}")),
        }
    }
}

impl ReadingDirection {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LeftToRight => "LEFT_TO_RIGHT",
            Self::RightToLeft => "RIGHT_TO_LEFT",
            Self::Vertical => "VERTICAL",
            Self::Webtoon => "WEBTOON",
        }
    }
}

/// Kotlin `SeriesMetadata.Status`, stored and sent by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SeriesStatus {
    Ended,
    Ongoing,
    Abandoned,
    Hiatus,
}

impl SeriesStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ended => "ENDED",
            Self::Ongoing => "ONGOING",
            Self::Abandoned => "ABANDONED",
            Self::Hiatus => "HIATUS",
        }
    }
}

impl FromStr for SeriesStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "ENDED" => Ok(Self::Ended),
            "ONGOING" => Ok(Self::Ongoing),
            "ABANDONED" => Ok(Self::Abandoned),
            "HIATUS" => Ok(Self::Hiatus),
            other => Err(format!("Unknown series status {other}")),
        }
    }
}

/// Kotlin `SeriesMetadata`. Each field has a lock that keeps automatic refreshes from changing it.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesMetadata {
    pub series_id: SeriesId,
    pub status: SeriesStatus,
    pub status_lock: bool,
    pub title: String,
    pub title_lock: bool,
    pub title_sort: String,
    pub title_sort_lock: bool,
    pub summary: String,
    pub summary_lock: bool,
    pub reading_direction: Option<ReadingDirection>,
    pub reading_direction_lock: bool,
    pub publisher: String,
    pub publisher_lock: bool,
    pub age_rating: Option<i32>,
    pub age_rating_lock: bool,
    /// A normalized BCP 47 tag, or empty.
    pub language: String,
    pub language_lock: bool,
    /// Lowercase, trimmed, non-blank, and distinct.
    pub genres: Vec<String>,
    pub genres_lock: bool,
    pub total_book_count: Option<i32>,
    pub total_book_count_lock: bool,
    /// Lowercase, trimmed, non-blank, and distinct.
    pub tags: Vec<String>,
    pub tags_lock: bool,
    /// Lowercase, trimmed, non-blank, and distinct.
    pub sharing_labels: Vec<String>,
    pub sharing_labels_lock: bool,
    pub links: Vec<WebLink>,
    pub links_lock: bool,
    pub alternate_titles: Vec<AlternateTitle>,
    pub alternate_titles_lock: bool,
}

/// Kotlin `AlternateTitle`: a title in another language or script, labeled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlternateTitle {
    pub label: String,
    pub title: String,
}

impl SeriesMetadata {
    /// The Kotlin constructor's normalization: title, title sort, summary, and publisher trimmed;
    /// genres, tags, and sharing labels lowercased, trimmed, non-blank, and distinct. Languages
    /// are normalized before they get here.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.title = self.title.trim().to_owned();
        self.title_sort = self.title_sort.trim().to_owned();
        self.summary = self.summary.trim().to_owned();
        self.publisher = self.publisher.trim().to_owned();
        self.genres = lower_not_blank(&self.genres);
        self.tags = lower_not_blank(&self.tags);
        self.sharing_labels = lower_not_blank(&self.sharing_labels);
        self
    }
}

crate::sql::sql_enum!(ReadingDirection => as_str, SeriesStatus => as_str);
