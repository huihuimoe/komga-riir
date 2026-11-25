//! Kotlin `BookMetadataLifecycle.refreshMetadata`: book metadata and read lists from the metadata
//! providers.

mod aggregation;
mod edit;
mod manual_refresh;
pub(crate) mod providers;
mod series_refresh;
pub(crate) mod transient;

use komga_db::{
    BookDao, BookMetadataDao, Database, DbError, LibraryDao, MediaDao, SearchIndexPendingDao,
};
use komga_scheduler::{Task, TaskQueue};
use komga_types::{BookId, BookMetadataPatchCapability, BookRef, DomainEvent, SearchEntity};
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::catalog::read_lists;
use crate::catalog::search_index::SearchIndexNotifier;
use crate::events::EventBus;
use crate::reading::ReadingService;
use edit::patch;
pub use edit::{BookMetadataUpdate, SeriesMetadataUpdate};
use providers::{BOOK_PROVIDERS, Target};

#[derive(Debug, Error)]
pub enum MetadataError {
    #[error("Not found")]
    NotFound,
    #[error("Forbidden")]
    Forbidden,
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error("Book {0} has no media")]
    MissingMedia(BookId),
}

#[derive(Clone)]
pub struct MetadataService {
    db: Database,
    tasks: TaskQueue,
    reading: ReadingService,
    search: SearchIndexNotifier,
    events: EventBus,
}

impl MetadataService {
    #[must_use]
    pub fn new(
        db: Database,
        tasks: TaskQueue,
        reading: ReadingService,
        search: SearchIndexNotifier,
        events: EventBus,
    ) -> Self {
        Self {
            db,
            tasks,
            reading,
            search,
            events,
        }
    }

    /// Applies the patches of the providers that support one of `capabilities`, then queues the
    /// refresh of the book's series one below `priority`, as Kotlin's `TaskHandler` does.
    /// Providers read the book outside any transaction; the patches are applied in one.
    pub(crate) async fn refresh_book_metadata(
        &self,
        id: &BookId,
        capabilities: &[BookMetadataPatchCapability],
        priority: i32,
    ) -> Result<(), MetadataError> {
        let mut tx = self.db.begin().await?;
        let Some(book) = BookDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            warn!(book = %id, "Cannot refresh metadata: Book does not exist");
            return Ok(());
        };
        let media = MediaDao::find_by_id(&mut tx, id)
            .await?
            .ok_or_else(|| MetadataError::MissingMedia(id.clone()))?;
        let library = LibraryDao::find_by_id(&mut tx, &book.library_id)
            .await?
            .expect("A book's library exists (foreign key)");
        tx.rollback().await?;
        info!(book = %id, url = %book.url, ?capabilities, "Refresh metadata for book");

        let mut patches = Vec::new();
        let mut read_lists = Vec::new();
        for provider in BOOK_PROVIDERS {
            let name = provider.name();
            if !provider
                .capabilities()
                .iter()
                .any(|capability| capabilities.contains(capability))
            {
                info!(
                    provider = name,
                    "Provider does not support requested capabilities, skipping"
                );
                continue;
            }
            let to_book = provider.handles(&library, Target::Book);
            let to_read_lists = provider.handles(&library, Target::ReadList);
            if !to_book && !to_read_lists {
                info!(
                    provider = name,
                    "Library is not set to import book or read lists metadata for this provider, \
                     skipping"
                );
                continue;
            }
            debug!(provider = name, "Provider");
            let Some(mut patch) = provider.book_patch(&self.reading, &book, &media).await else {
                continue;
            };
            if to_read_lists {
                read_lists.append(&mut patch.read_lists);
            }
            if to_book {
                patches.push(patch);
            }
        }

        if !patches.is_empty() || !read_lists.is_empty() {
            let mut tx = self.db.begin_immediate().await?;
            let Some(metadata) = BookMetadataDao::find_by_id(&mut tx, id).await? else {
                tx.rollback().await?;
                return Ok(());
            };
            if !patches.is_empty() {
                let patched = patches
                    .iter()
                    .fold(metadata, |metadata, found| patch::apply(found, metadata));
                BookMetadataDao::update(&mut tx, &patched).await?;
                SearchIndexPendingDao::mark(&mut tx, SearchEntity::Book, &[id.as_str()]).await?;
                tx.record(DomainEvent::BookUpdated(BookRef::from(&book)));
            }
            for entry in &read_lists {
                read_lists::add_book(&mut tx, &entry.name, &book, entry.number).await?;
            }
            self.events.commit(tx).await?;
            self.search.notify();
        }
        self.tasks
            .submit(&Task::refresh_series_metadata(book.series_id).with_priority(priority - 1))
            .await?;
        Ok(())
    }
}
