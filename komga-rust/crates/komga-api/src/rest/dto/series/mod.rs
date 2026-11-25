//! Series, metadata update, and series search DTOs.

mod details;
mod series_metadata_update;
mod series_search;

pub use details::SeriesDto;
pub use series_metadata_update::SeriesMetadataUpdateDto;
pub use series_search::{GroupCountDto, SeriesSearchDto};
