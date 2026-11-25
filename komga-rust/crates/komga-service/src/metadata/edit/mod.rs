//! Admin metadata edits and provider patch application for books and series.

mod book_edit;
pub(super) mod patch;
mod series_edit;
pub(super) mod series_patch;

pub use book_edit::BookMetadataUpdate;
pub use series_edit::SeriesMetadataUpdate;
