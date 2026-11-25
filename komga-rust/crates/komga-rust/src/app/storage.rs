//! Data directory locking, database migrations, search index opening, and startup settings.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use komga_db::{Database, DirectoryLock, LibraryDao};
use komga_search::SearchIndex;
use komga_service::{EventBus, SearchIndexNotifier, ServerSettings};
use tracing::{info, warn};

use super::local_filesystem;
use crate::config::{AppConfig, DATABASE_CHECK_LOCAL_FILESYSTEM, TASKS_DB_CHECK_LOCAL_FILESYSTEM};

/// How long startup waits for an existing instance to release the data directory lock.
/// Exceeds `SHUTDOWN_TIMEOUT` so a replacement instance can take over once the previous one
/// finishes shutting down.
pub(super) const LOCK_WAIT_TIMEOUT: Duration = Duration::from_secs(35);

/// Poll interval while waiting for the data directory lock to become available.
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// The locked data directory, its migrated databases and search index, and the settings read at
/// startup.
pub(super) struct Storage {
    _directory_lock: DirectoryLock,
    pub(super) search_index: Arc<SearchIndex>,
    /// Wakes the updater of `search_index` after changes.
    pub(super) search: SearchIndexNotifier,
    /// Committed changes, for event streams.
    pub(super) events: EventBus,
    pub(super) database: Database,
    pub(super) tasks: Database,
    pub(super) settings: ServerSettings,
}

impl Storage {
    pub(super) async fn open(
        config: &AppConfig,
        lock_wait_timeout: Duration,
        shutdown: &mut (impl Future<Output = ()> + Unpin),
    ) -> Result<Option<Self>> {
        if config.check_local_filesystem.value {
            local_filesystem::check_database_is_local(
                &config.database_file.value,
                DATABASE_CHECK_LOCAL_FILESYSTEM,
            )?;
        }
        if config.tasks_check_local_filesystem.value {
            local_filesystem::check_database_is_local(
                &config.tasks_database_file.value,
                TASKS_DB_CHECK_LOCAL_FILESYSTEM,
            )?;
        }
        let Some(directory_lock) =
            acquire_directory_lock(&config.config_dir.value, lock_wait_timeout, shutdown).await?
        else {
            return Ok(None);
        };
        let database =
            Database::connect_sorting_by(&config.database_file.value, config.sort.rules())
                .await
                .context("Failed to connect to the Komga database")?;
        let tasks = Database::connect(&config.tasks_database_file.value)
            .await
            .context("Failed to connect to the tasks database")?;
        Database::migrate_all(&database, &tasks, &config.config_dir.value.join("backups"))
            .await
            .context("Failed to migrate the databases")?;
        warn_of_unknown_sorted_libraries(&database, config).await?;
        let lucene = config.lucene.clone();
        let search_index = tokio::task::spawn_blocking(move || {
            SearchIndex::open(&lucene.data_directory.value, lucene.ngram())
        })
        .await
        .context("Search index opening panicked")?
        .context("Failed to open the search index")?;
        let settings = ServerSettings::load(&database)
            .await
            .context("Failed to load server settings")?;
        Ok(Some(Self {
            _directory_lock: directory_lock,
            search_index: Arc::new(search_index),
            search: SearchIndexNotifier::default(),
            events: EventBus::default(),
            database,
            tasks,
            settings,
        }))
    }
}

async fn acquire_directory_lock(
    dir: &Path,
    timeout: Duration,
    shutdown: &mut (impl Future<Output = ()> + Unpin),
) -> Result<Option<DirectoryLock>> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut waited = false;

    loop {
        if let Some(lock) = DirectoryLock::try_acquire(dir)
            .context("Failed to acquire Komga data directory lock")?
        {
            if waited {
                info!(dir = %dir.display(), "Acquired Komga data directory lock");
            }
            return Ok(Some(lock));
        }

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            bail!(
                "Directory is locked by another instance after waiting {timeout:?}: {}",
                dir.display()
            );
        }

        if !waited {
            warn!(
                dir = %dir.display(),
                timeout = ?timeout,
                "Data directory is locked by another instance; waiting for it to be released"
            );
            waited = true;
        }

        tokio::select! {
            biased;
            () = &mut *shutdown => {
                info!("Shutdown requested while waiting for Komga data directory lock");
                return Ok(None);
            }
            () = tokio::time::sleep(remaining.min(LOCK_POLL_INTERVAL)) => {}
        }
    }
}

/// A sort locale for a library that does not exist orders nothing, which a mistyped ID would
/// otherwise hide.
async fn warn_of_unknown_sorted_libraries(database: &Database, config: &AppConfig) -> Result<()> {
    let mut transaction = database.begin().await?;
    for library in config.sort.library_locales.value.keys() {
        if LibraryDao::find_by_id(&mut transaction, library)
            .await?
            .is_none()
        {
            warn!(%library, "komga.sort.library-locales names a library that does not exist");
        }
    }
    Ok(())
}
