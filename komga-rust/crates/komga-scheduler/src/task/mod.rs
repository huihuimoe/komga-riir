//! Kotlin `Task`: queued work, its deduplication key, and its persisted encoding.

use komga_db::TaskRow;
use komga_types::{
    BookId, BookMetadataPatchCapability, BookPageNumbered, ImportRequest, LibraryId, SeriesId,
};

mod macros;

pub const HIGHEST_PRIORITY: i32 = 8;
pub const HIGH_PRIORITY: i32 = 6;
pub const DEFAULT_PRIORITY: i32 = 4;
pub const LOW_PRIORITY: i32 = 2;
pub const LOWEST_PRIORITY: i32 = 0;

/// Rows keep the Kotlin class name and Jackson payload so that tasks left by the Kotlin server
/// stay readable.
const CLASS_PREFIX: &str = "org.gotson.komga.application.tasks.Task$";

/// A queued task paired with its scheduling priority.
#[must_use = "tasks do nothing until submitted to a TaskQueue"]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub kind: TaskKind,
    pub priority: i32,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TaskDecodeError {
    #[error("unsupported task class {0}")]
    UnsupportedClass(String),
    #[error("invalid {class} payload: {source}")]
    Payload {
        class: String,
        source: serde_json::Error,
    },
}

macros::define_tasks! {
    EmptyTrash(empty_trash) {
        library_id: LibraryId,
    } => {
        id: format!("EMPTY_TRASH_{library_id}"),
    },
    ScanLibrary(scan_library) {
        library_id: LibraryId,
        scan_deep: bool,
    } => {
        id: format!("SCAN_LIBRARY_{library_id}_DEEP_{scan_deep}"),
    },
    FindBooksToConvert(find_books_to_convert) {
        library_id: LibraryId,
    } => {
        id: format!("FIND_BOOKS_TO_CONVERT_{library_id}"),
    },
    FindBooksWithMissingPageHash(find_books_with_missing_page_hash) {
        library_id: LibraryId,
    } => {
        id: format!("FIND_BOOKS_WITH_MISSING_PAGE_HASH_{library_id}"),
    },
    FindDuplicatePagesToDelete(find_duplicate_pages_to_delete) {
        library_id: LibraryId,
    } => {
        id: format!("FIND_DUPLICATE_PAGES_TO_DELETE_{library_id}"),
    },
    AnalyzeBook(analyze_book) {
        book_id: BookId,
        #[serde(rename = "groupId")]
        series_id: SeriesId,
    } => {
        id: format!("ANALYZE_BOOK_{book_id}"),
        group: series_id,
    },
    GenerateBookThumbnail(generate_book_thumbnail) {
        book_id: BookId,
    } => {
        id: format!("GENERATE_BOOK_THUMBNAIL_{book_id}"),
    },
    ConvertBook(convert_book) {
        book_id: BookId,
        #[serde(rename = "groupId")]
        series_id: SeriesId,
    } => {
        id: format!("CONVERT_BOOK_{book_id}"),
        group: series_id,
    },
    RepairExtension(repair_extension) {
        book_id: BookId,
        #[serde(rename = "groupId")]
        series_id: SeriesId,
    } => {
        id: format!("REPAIR_EXTENSION_{book_id}"),
        group: series_id,
    },
    RemoveHashedPages(remove_hashed_pages) {
        book_id: BookId,
        pages: Vec<BookPageNumbered>,
    } => {
        id: format!("REMOVE_HASHED_PAGES_{book_id}"),
    },
    DeleteBook(delete_book) {
        book_id: BookId,
    } => {
        id: format!("DELETE_BOOK_{book_id}"),
    },
    DeleteSeries(delete_series) {
        series_id: SeriesId,
    } => {
        id: format!("DELETE_SERIES_{series_id}"),
    },
    HashBook(hash_book) {
        book_id: BookId,
    } => {
        id: format!("HASH_BOOK_{book_id}"),
    },
    HashBookKoreader(hash_book_koreader) {
        book_id: BookId,
    } => {
        id: format!("HASH_BOOK_KOREADER_{book_id}"),
    },
    HashBookPages(hash_book_pages) {
        book_id: BookId,
    } => {
        id: format!("HASH_BOOK_PAGES_{book_id}"),
    },
    RefreshBookMetadata(refresh_book_metadata) {
        book_id: BookId,
        #[serde(rename = "groupId")]
        series_id: SeriesId,
        capabilities: Vec<BookMetadataPatchCapability>,
    } => {
        id: format!("REFRESH_BOOK_METADATA_{book_id}"),
        group: series_id,
    },
    RefreshSeriesMetadata(refresh_series_metadata) {
        series_id: SeriesId,
    } => {
        id: format!("REFRESH_SERIES_METADATA_{series_id}"),
        group: series_id,
    },
    AggregateSeriesMetadata(aggregate_series_metadata) {
        series_id: SeriesId,
    } => {
        id: format!("AGGREGATE_SERIES_METADATA_{series_id}"),
        group: series_id,
    },
    RefreshBookLocalArtwork(refresh_book_local_artwork) {
        book_id: BookId,
    } => {
        id: format!("REFRESH_BOOK_LOCAL_ARTWORK_{book_id}"),
    },
    RefreshSeriesLocalArtwork(refresh_series_local_artwork) {
        series_id: SeriesId,
    } => {
        id: format!("REFRESH_SERIES_LOCAL_ARTWORK_{series_id}"),
    },
    FindBookThumbnailsToRegenerate(find_book_thumbnails_to_regenerate) {
        for_bigger_result_only: bool,
    } => {
        id: "FIND_BOOK_THUMBNAILS_TO_REGENERATE".to_owned(),
    },
    /// Rebuilds every entity: Kotlin's choice of entities only spares Lucene work.
    RebuildIndex(rebuild_index) => {
        id: "REBUILD_INDEX".to_owned(),
        wire_extra: { entities: Option<()> = None },
    },
    UpgradeIndex(upgrade_index) => {
        id: "UPGRADE_INDEX".to_owned(),
    },
    ImportBook(import_book) {
        #[serde(flatten)]
        request: ImportRequest,
    } => {
        id: format!("IMPORT_BOOK_{}_{}", request.series_id, request.source_file),
        group: &request.series_id,
    },
}

impl Task {
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub const fn highest(self) -> Self {
        self.with_priority(HIGHEST_PRIORITY)
    }

    pub const fn high(self) -> Self {
        self.with_priority(HIGH_PRIORITY)
    }

    pub const fn low(self) -> Self {
        self.with_priority(LOW_PRIORITY)
    }

    pub const fn lowest(self) -> Self {
        self.with_priority(LOWEST_PRIORITY)
    }

    pub(crate) fn simple_type(&self) -> &'static str {
        self.kind.simple_type()
    }

    pub(crate) fn to_row(&self) -> TaskRow {
        let simple_type = self.simple_type();
        TaskRow {
            id: self.kind.unique_id(),
            priority: self.priority,
            group_id: self.kind.group_id().map(ToString::to_string),
            class: format!("{CLASS_PREFIX}{simple_type}"),
            simple_type: simple_type.to_owned(),
            payload: self
                .kind
                .encode_payload(self.priority)
                .expect("Task payloads serialize to JSON"),
        }
    }

    pub(crate) fn decode(class: &str, raw: &str) -> Result<Self, TaskDecodeError> {
        TaskKind::decode_task(class, raw)
    }
}

#[cfg(test)]
mod tests;
