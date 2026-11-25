//! Book aggregate, metadata, media, read progress, and search conditions.

mod details;
mod dimension;
mod entity;
mod media;
pub(crate) mod metadata;
mod metadata_aggregation;
mod read_progress;
mod search;
mod thumbnail;

pub use details::{BookDetails, MediaDetails, ReadProgressDetails};
pub use dimension::Dimension;
pub use entity::{Book, CopyMode, ImportRequest};
pub use media::{
    BookPage, BookPageNumbered, EpubTocEntry, Media, MediaExtensionEpub, MediaFile,
    MediaFileSubType, MediaProfile, MediaStatus, R2Location, R2Locator, R2Text,
};
pub use metadata::{BookAuthor, BookMetadata, BookMetadataPatchCapability, NotableEntity, WebLink};
pub use metadata_aggregation::BookMetadataAggregation;
pub use read_progress::{R2Device, R2Progression, ReadProgress};
pub use search::{
    AuthorMatch, BookCondition, BookSort, NotableEntityMatch, PosterMatch, ReadStatus,
};
pub use thumbnail::{ThumbnailDetails, ThumbnailType};
