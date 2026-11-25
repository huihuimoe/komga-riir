//! Payloads of the events, as Kotlin `interfaces/sse/dto` serializes them.

mod book;
mod collection;
mod library;
mod read_list;
mod read_progress;
mod series;
mod session;
mod task;
mod thumbnail;

pub(super) use book::{BookImportSseDto, BookSseDto};
pub(super) use collection::CollectionSseDto;
pub(super) use library::LibrarySseDto;
pub(super) use read_list::ReadListSseDto;
pub(super) use read_progress::{ReadProgressSeriesSseDto, ReadProgressSseDto};
pub(super) use series::SeriesSseDto;
pub(super) use session::SessionExpiredSseDto;
pub(super) use task::TaskQueueSseDto;
pub(super) use thumbnail::{
    ThumbnailBookSseDto, ThumbnailReadListSseDto, ThumbnailSeriesCollectionSseDto,
    ThumbnailSeriesSseDto,
};
