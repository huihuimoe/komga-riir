//! Business services, use cases, and transactional orchestration for Komga.

mod artwork;
mod books;
mod catalog;
mod events;
mod identity;
mod library;
mod metadata;
mod reading;
mod server;
mod settings;
mod support;
mod sync;
mod tasks;

// Catalog, collections, read lists, and metadata
pub use artwork::{ArtworkError, ArtworkService};
pub use books::analysis::{BookAnalysisError, BookAnalysisService};
pub use books::hashing::{PageHashError, PageHashService};
pub use books::importing::BookImporter;
pub use books::transient::{TransientBook, TransientBookError, TransientBookService};
pub use catalog::collections::{
    CollectionError, CollectionUpdate, NewCollection, SeriesCollectionService,
};
pub use catalog::query::{CatalogError, CatalogService};
pub use catalog::read_lists::{
    NewReadList, ReadListError, ReadListMatch, ReadListRequestMatch, ReadListService,
    ReadListUpdate,
};
pub use catalog::referential::ReferentialService;
pub use catalog::search_index::{
    FullTextSearch, SearchBudgetError, SearchIndexError, SearchIndexNotifier, SearchIndexUpdater,
};
pub use library::file_deletion::FileDeletion;
pub use library::file_operation::FileOperations;
pub use library::service::{LibraryError, LibraryService, LibraryUpdate};
pub use metadata::{BookMetadataUpdate, MetadataError, MetadataService, SeriesMetadataUpdate};
pub use support::book_archive::BookArchive;

// Reading, progress, and sync protocols
pub use reading::fonts::{FontBytes, FontError, FontService};
pub use reading::progress::{
    ReadListTachiyomiProgress, ReadProgressError, ReadProgressService, SeriesTachiyomiProgress,
};
pub use reading::{
    BookFile, EpubResource, ManifestSource, PageContent, ReadingError, ReadingService,
    can_convert_to_jpeg, is_epub_font,
};
pub use sync::kobo::{
    KoboChanges, KoboError, KoboReadingUpdate, KoboReply, KoboSettings, KoboSync, KoboSyncService,
};
pub use sync::koreader::{KoreaderError, KoreaderProgress, KoreaderSyncService};
pub use sync::points::SyncPointService;

// Identity and access
pub use identity::external::{
    ExternalIdentityService, FlowKeyRing, FlowSecret, SignInError, SignInPolicy,
};
pub use identity::{
    DEFAULT_SESSION_TTL, IdentityError, IdentityService, LibrarySharing, NewUser, SessionRegistry,
    UserUpdate,
};

// Settings, events, tasks, and server operations
pub use events::{EventBus, EventFeed};
pub use server::announcements::{AnnouncementError, AnnouncementService, JsonFeed};
pub use server::filesystem::{
    DirectoryEntry, DirectoryListing, FilesystemError, FilesystemService,
};
pub use server::health::{DISK_SPACE_THRESHOLD, Health, HealthService};
pub use server::history::HistoryService;
pub use server::metrics::{Metric, MetricsService, Statistic};
pub use server::releases::{Release, ReleaseService};
pub use settings::{
    ClientSettingsService, RememberMeSettings, ServerSettings, ServerSettingsService,
    ServerSettingsUpdate,
};
pub use support::blocking::BlockingError;
pub use tasks::{TaskError, TaskHandler, TaskHandlerConfig, TaskService};

/// Query results returned as they are, so that `komga-api` needs no `komga-db` dependency.
pub use komga_db::{Credit, PosterSet, SeriesValue, Sibling};
