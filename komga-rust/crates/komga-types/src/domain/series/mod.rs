//! Series aggregate, metadata, and search conditions.

mod details;
mod entity;
mod metadata;
mod search;

pub use details::SeriesDetails;
pub use entity::Series;
pub use metadata::{AlternateTitle, ReadingDirection, SeriesMetadata, SeriesStatus};
pub use search::{GroupCount, SeriesCondition, SeriesSort, TitleField, TitleRegex};
