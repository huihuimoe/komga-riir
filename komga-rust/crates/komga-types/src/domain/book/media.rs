//! Kotlin `Media`, `BookPage`, and `MediaFile`: what analysis found in a book file.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::domain::Dimension;

/// Kotlin `Media.Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MediaStatus {
    Unknown,
    Error,
    Ready,
    Unsupported,
    Outdated,
}

impl MediaStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::Error => "ERROR",
            Self::Ready => "READY",
            Self::Unsupported => "UNSUPPORTED",
            Self::Outdated => "OUTDATED",
        }
    }
}

impl fmt::Display for MediaStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for MediaStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "UNKNOWN" => Ok(Self::Unknown),
            "ERROR" => Ok(Self::Error),
            "READY" => Ok(Self::Ready),
            "UNSUPPORTED" => Ok(Self::Unsupported),
            "OUTDATED" => Ok(Self::Outdated),
            other => Err(format!("Unknown media status {other}")),
        }
    }
}

/// Kotlin `Media`.
#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    pub status: MediaStatus,
    pub media_type: Option<String>,
    pub pages: Vec<BookPage>,
    /// The page count of formats whose pages are not listed (EPUB); `pages.len()` otherwise.
    pub page_count: i32,
    pub files: Vec<MediaFile>,
    pub comment: Option<String>,
    pub epub_divina_compatible: bool,
    pub epub_is_kepub: bool,
    /// `None` keeps the stored extension, as Kotlin does for a media read back from the database
    /// or analyzed as another format.
    pub extension: Option<MediaExtensionEpub>,
}

impl Media {
    /// Kotlin `Media(status = status, comment = comment)`: no type, pages, or files.
    #[must_use]
    pub fn failed(status: MediaStatus, comment: Option<String>) -> Self {
        Self {
            status,
            media_type: None,
            pages: Vec::new(),
            page_count: 0,
            files: Vec::new(),
            comment,
            epub_divina_compatible: false,
            epub_is_kepub: false,
            extension: None,
        }
    }
}

/// Kotlin `MediaProfile`: how a book is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum MediaProfile {
    Divina,
    Pdf,
    Epub,
}

impl MediaProfile {
    /// `MediaType.fromMediaType(mediaType)?.profile`: `None` for other or missing types.
    #[must_use]
    pub fn from_media_type(media_type: Option<&str>) -> Option<Self> {
        let media_type = media_type?;
        [Self::Divina, Self::Pdf, Self::Epub]
            .into_iter()
            .find(|profile| profile.media_types().contains(&media_type))
    }

    /// `MediaType.matchingMediaProfile`: the media types read this way.
    #[must_use]
    pub fn media_types(self) -> &'static [&'static str] {
        match self {
            Self::Divina => &[
                "application/zip",
                "application/x-rar-compressed",
                "application/x-rar-compressed; version=4",
                "application/x-rar-compressed; version=5",
            ],
            Self::Pdf => &["application/pdf"],
            Self::Epub => &["application/epub+zip", "application/x-mobipocket-ebook"],
        }
    }
}

/// Kotlin `BookPage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookPage {
    pub file_name: String,
    pub media_type: String,
    pub dimension: Option<Dimension>,
    /// Empty until page hashing.
    pub file_hash: String,
    pub file_size: Option<i64>,
}

/// Kotlin `BookPageNumbered`: a page of a book with its number, from 1. The `RemoveHashedPages`
/// task payload carries these in Jackson's form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookPageNumbered {
    pub file_name: String,
    pub media_type: String,
    pub dimension: Option<Dimension>,
    pub file_hash: String,
    pub file_size: Option<i64>,
    pub page_number: i32,
}

/// Kotlin `MediaFile`: a member of the book file that is not a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFile {
    pub file_name: String,
    /// `None` when the member could not be read.
    pub media_type: Option<String>,
    pub sub_type: Option<MediaFileSubType>,
    pub file_size: Option<i64>,
}

/// Kotlin `MediaFile.SubType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFileSubType {
    EpubPage,
    EpubAsset,
}

impl MediaFileSubType {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EpubPage => "EPUB_PAGE",
            Self::EpubAsset => "EPUB_ASSET",
        }
    }
}

impl FromStr for MediaFileSubType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "EPUB_PAGE" => Ok(Self::EpubPage),
            "EPUB_ASSET" => Ok(Self::EpubAsset),
            other => Err(format!("Unknown media file sub type {other}")),
        }
    }
}

/// Kotlin `MediaExtensionEpub`, serialized as Komga's Jackson mapper does: every property, `null`
/// included. Missing properties read as the Kotlin defaults; unknown ones are ignored, as Spring
/// Boot's mapper does.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MediaExtensionEpub {
    pub toc: Vec<EpubTocEntry>,
    pub landmarks: Vec<EpubTocEntry>,
    pub page_list: Vec<EpubTocEntry>,
    /// jackson-module-kotlin keeps the `is` prefix of Kotlin boolean properties.
    #[serde(rename = "isFixedLayout")]
    pub is_fixed_layout: bool,
    pub positions: Vec<R2Locator>,
}

/// Kotlin `R2Locator`, whose `@JsonInclude(NON_EMPTY)` omits `null`, empty strings, and empty
/// lists.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct R2Locator {
    #[serde(skip_serializing_if = "str::is_empty")]
    pub href: String,
    #[serde(rename = "type", skip_serializing_if = "str::is_empty")]
    pub media_type: String,
    #[serde(skip_serializing_if = "is_null_or_empty")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locations: Option<R2Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<R2Text>,
    /// Komga's mapping to the `koboSpan` id at this position.
    #[serde(skip_serializing_if = "is_null_or_empty")]
    pub kobo_span: Option<String>,
}

/// Kotlin `R2Locator.Location`, `NON_EMPTY` as well.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct R2Location {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progression: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_progression: Option<f32>,
}

/// Kotlin `R2Locator.Text`, `NON_EMPTY` as well.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct R2Text {
    #[serde(skip_serializing_if = "is_null_or_empty")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "is_null_or_empty")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "is_null_or_empty")]
    pub highlight: Option<String>,
}

#[expect(clippy::ref_option, reason = "serde passes a reference to the field")]
fn is_null_or_empty(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(str::is_empty)
}

/// Kotlin `EpubTocEntry`: an entry of an EPUB table of contents, landmarks, or page list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpubTocEntry {
    pub title: String,
    /// The member name, with its anchor.
    pub href: Option<String>,
    #[serde(default)]
    pub children: Vec<EpubTocEntry>,
}

crate::sql::sql_enum!(MediaStatus => as_str, MediaFileSubType => as_str);
