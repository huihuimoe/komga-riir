//! Book, page, read progress, and thumbnail DTOs.

mod book_metadata_update;
mod book_search;
mod details;
mod page;
mod r2_positions;
mod r2_progression;
mod read_progress_update;
mod tachiyomi_read_progress;
mod thumbnail;

pub use book_metadata_update::BookMetadataUpdateDto;
pub(in crate::rest::dto) use book_metadata_update::{
    WebLinkUpdateDto, not_blank_pairs, validated, web_link_violations, web_links,
};
pub use book_search::BookSearchDto;
pub use details::BookDto;
pub(crate) use details::{AuthorDto, NotableEntityDto, WebLinkDto};
pub use page::PageDto;
pub use r2_positions::R2Positions;
pub use r2_progression::R2ProgressionDto;
pub use read_progress_update::{ReadProgressMark, ReadProgressUpdateDto};
pub use tachiyomi_read_progress::{
    TachiyomiReadProgressDto, TachiyomiReadProgressUpdateDto, TachiyomiReadProgressUpdateV2Dto,
    TachiyomiReadProgressV2Dto,
};
pub use thumbnail::{
    ThumbnailBookDto, ThumbnailReadListDto, ThumbnailSeriesCollectionDto, ThumbnailSeriesDto,
};
