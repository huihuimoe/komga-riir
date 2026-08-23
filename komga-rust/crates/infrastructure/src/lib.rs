mod discovery;
mod event_emitter_adapter;
mod filesystem;
mod identity;
mod media;
mod metadata;
mod opds;
mod operational;
mod persistence;
mod progress_writer;
mod search;
mod search_sync_adapter;
mod shared;
mod sql;
mod task_enqueue_adapter;
mod task_queue;
#[cfg(test)]
pub(crate) mod test_support;
mod thumbnail_writer;

pub use discovery::{
    DiscoveryDetailAccess, DiscoveryQuerySupportAccess, LibraryCatalogAccess,
    SqliteDiscoveryBrowseService,
};
pub use event_emitter_adapter::SseBookEventEmitter;
pub use filesystem::{FilesystemBookImport, remove_file_after_release};
pub use identity::{
    ClaimAccess, IdentityAccess, InitialBootstrapUserWriteModel, PersistedBootstrapUser,
    invalidate_user_sessions, list_persisted_user_emails, load_persisted_user_by_email,
    load_persisted_user_count, persist_initial_bootstrap_users,
    persisted_update_password_by_user_id, update_persisted_user_passwords,
};
pub use media::{ContentResolver, MediaReader, ZipArchiveBuilder};
pub use metadata::{SqliteBookMetadataPort, generate_book_thumbnail};
pub use opds::{OpdsCatalogAccess, OpdsPersistedAccess};
pub use operational::{
    ActuatorSnapshotAccess, AnnouncementAccess, ClientSettingsAccess, FilesystemBrowseAccess,
    FontAccess, HistoryAccess, OperationalMetricsAccess, PageHashAccess, RememberMeRuntimeSettings,
    RemoteFeedAccess, ServerSettingsStore, SyncpointAccess, TransientBookAccess,
    load_remember_me_runtime_settings,
};
pub use persistence::{
    DEFAULT_MAX_CONNECTIONS, DatabaseHandle, SharedSqlitePoolSnapshot, SqlitePersistenceConnection,
    SqlitePersistenceContext, SqliteTempPool, SqliteUnitOfWork, WRITE_MAX_CONNECTIONS,
    bootstrap_pool, bootstrap_tasks_pool, close_all_shared_pools, connect_main_write_context,
    connect_read_pool, connect_shared_pool, connect_task_pool, connect_task_write_pool,
    connect_write_pool, default_read_max_connections, evict_shared_pools_for_paths,
    file_backed_connect_options, reject_or_quarantine_pool_topology,
    shared_pool_snapshots_for_paths,
};
pub use progress_writer::ProgressWriter;
pub use search::{
    SearchEntityType, SearchIndexLifecycle, SearchStartupLifecycle, decide_startup_lifecycle,
    prepare_for_rebuild, rebuild_index_from_database, search_analyzer_version,
};
pub use search_sync_adapter::SearchSyncAdapter;
pub use task_enqueue_adapter::TaskEnqueueAdapter;
pub use task_queue::{
    DatabaseRuntime, FilesystemRuntime, JobRuntime, RuntimeBackgroundState, SearchRuntime,
    SharedTaskQueue, SqliteFilesystemLibraryScanPipeline, TaskQueueScheduler, TaskQueueWakeSignal,
    TaskRuntimeConfig, TaskRuntimeContext, TaskRuntimeOwnershipOverrides, WorkerRuntime,
    cleanup_authentication_activity_once, prepare_task_queue, process_startup_library_scans,
    run_background_task_iteration, run_periodic_library_scan_iteration,
};
pub use thumbnail_writer::ThumbnailWriter;

pub(crate) use persistence::{
    resolve_library_item_path, resolve_optional_library_item_path, resolve_rooted_path,
    resolve_stored_path,
};
pub(crate) use shared::random_hex_token;
