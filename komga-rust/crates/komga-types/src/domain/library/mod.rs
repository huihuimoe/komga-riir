//! Library aggregate, sidecars, thumbnails, and page hashes.

mod entity;
mod page_hash;
mod sidecar;
mod thumbnail_size;

pub use entity::{Library, ScanInterval, SeriesCover};
pub use page_hash::{
    KnownPageHashSort, PageHashAction, PageHashKnown, PageHashMatch, PageHashMatchSort,
    PageHashUnknown, UnknownPageHashSort,
};
pub use sidecar::{Sidecar, SidecarKind};
pub use thumbnail_size::ThumbnailSize;
