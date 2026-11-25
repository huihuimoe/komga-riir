//! SQLite data access objects.

mod announcements_read;
mod authentication_activity;
pub(crate) mod book;
mod client_settings;
mod entity_count;
mod file_operation;
mod historical_event;
mod kobo;
mod library;
mod media;
mod page_hash;
mod read_list;
mod read_progress;
mod referential;
mod search_fields;
mod search_index_pending;
pub(crate) mod series;
mod series_collection;
mod server_settings;
mod sidecar;
mod sync_point;
mod task_handover;
mod tasks;
mod thumbnail;
mod user;

pub use announcements_read::AnnouncementsReadDao;
pub use authentication_activity::AuthenticationActivityDao;
pub use book::{
    BookDao, BookDtoDao, BookHash, BookMetadataAggregationDao, BookMetadataDao, BookNumbering,
    Sibling,
};
pub use client_settings::ClientSettingsDao;
pub use entity_count::{EntityCountDao, LibraryCount, TotalCount};
pub use file_operation::{FileOperation, FileOperationDao, FileVersion};
pub use historical_event::HistoricalEventDao;
pub use kobo::KoboDao;
pub use library::{DeletedContent, LibraryDao, RestoreDao};
pub use media::MediaDao;
pub use page_hash::PageHashDao;
pub use read_list::{ReadListBooks, ReadListContent, ReadListDao, ReadListRequestDao};
pub use read_progress::{NumberedCompletion, ReadProgressDao, ReadProgressDtoDao};
pub use referential::{Credit, ReferentialDao, SeriesValue};
pub use search_fields::{BookSearchFields, OneshotSeriesFields, SeriesSearchFields};
pub use search_index_pending::{PendingUpdate, SearchIndexPendingDao};
pub use series::{SeriesDao, SeriesDtoDao, SeriesMetadataDao};
pub use series_collection::{CollectionSeries, SeriesCollectionDao};
pub use server_settings::ServerSettingsDao;
pub use sidecar::{SidecarDao, StoredSidecar};
pub use sync_point::SyncPointDao;
pub use task_handover::TaskHandoverDao;
pub use tasks::{ClaimedTask, TaskEnd, TaskRow, TasksDao};
pub use thumbnail::{
    NewThumbnail, PosterSet, ThumbnailBookDao, ThumbnailContent, ThumbnailEntry,
    ThumbnailSeriesDao, ThumbnailSetDao,
};
pub use user::KomgaUserDao;
