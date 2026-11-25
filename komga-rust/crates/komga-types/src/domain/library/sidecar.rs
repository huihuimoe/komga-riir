//! Kotlin `Sidecar`: a file next to a series or book that a provider reads.

use jiff::civil::DateTime;

use crate::domain::FileUrl;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sidecar {
    pub url: FileUrl,
    /// The series directory, or the book file a book sidecar belongs to.
    pub parent_url: FileUrl,
    /// Server local time, like the Kotlin `LocalDateTime`.
    pub last_modified_time: DateTime,
    pub kind: SidecarKind,
}

/// Kotlin `Sidecar.Type` and `Sidecar.Source` together; no Kotlin consumer reads book metadata
/// sidecars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarKind {
    /// `LocalArtworkProvider` series covers such as `cover.jpg`.
    SeriesArtwork,
    /// `MylarSeriesProvider`'s `series.json`.
    SeriesMetadata,
    /// `LocalArtworkProvider` book covers such as `<book>-1.png`.
    BookArtwork,
}
