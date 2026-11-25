//! Kotlin `SeriesMetadataLifecycle.refreshMetadata`: series metadata and collections from the
//! metadata providers.

use komga_db::{
    BookDao, BookMetadataDao, LibraryDao, MediaDao, SearchIndexPendingDao, SeriesDao,
    SeriesMetadataDao,
};
use komga_scheduler::Task;
use komga_types::{Book, BookMetadata, DomainEvent, Library, Media, Series, SeriesId, SeriesRef};
use tracing::{debug, info, warn};

use super::edit::series_patch::{self, SeriesMetadataPatch};
use super::providers::{SERIES_FROM_BOOK_PROVIDERS, SERIES_PROVIDERS, Target};
use super::{MetadataError, MetadataService};
use crate::catalog::collections;

impl MetadataService {
    /// Applies what the providers find for the series, then queues the aggregation of its book
    /// metadata at `priority`, as Kotlin's `TaskHandler` does. Providers read the books outside
    /// any transaction; the patches are applied in one.
    pub(crate) async fn refresh_series_metadata(
        &self,
        id: &SeriesId,
        priority: i32,
    ) -> Result<(), MetadataError> {
        let Some((series, library, books)) = self.load_series(id).await? else {
            warn!(series = %id, "Cannot refresh metadata: Series does not exist");
            return Ok(());
        };
        info!(series = %id, url = %series.url, "Refresh metadata for series");
        let (mut patches, collections) = from_books(&library, &books).await;
        let first_book = match books.first() {
            Some((book, _)) if series.oneshot => self.book_metadata(book).await?,
            _ => None,
        };
        for provider in SERIES_PROVIDERS {
            let name = provider.name();
            if !provider.handles_series(&library) {
                info!(
                    provider = name,
                    "Library is not set to import series metadata for this provider, skipping"
                );
                continue;
            }
            debug!(provider = name, "Provider");
            patches.extend(provider.series_patch(&series, first_book.as_ref()).await);
        }

        let mut tx = self.db.begin_immediate().await?;
        let Some(metadata) = SeriesMetadataDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            return Ok(());
        };
        if !patches.is_empty() {
            let patched = patches.iter().fold(metadata, |metadata, found| {
                series_patch::apply(found, metadata)
            });
            SeriesMetadataDao::update(&mut tx, &patched).await?;
            SearchIndexPendingDao::mark_series_metadata(&mut tx, id).await?;
            tx.record(DomainEvent::SeriesUpdated(SeriesRef::from(&series)));
        }
        for name in &collections {
            collections::add_series(&mut tx, name, &series).await?;
        }
        self.events.commit(tx).await?;
        self.search.notify();
        self.tasks
            .submit(&Task::aggregate_series_metadata(series.id).with_priority(priority))
            .await?;
        Ok(())
    }

    async fn book_metadata(&self, book: &Book) -> Result<Option<BookMetadata>, MetadataError> {
        let mut tx = self.db.begin().await?;
        let metadata = BookMetadataDao::find_by_id(&mut tx, &book.id).await?;
        tx.rollback().await?;
        Ok(metadata)
    }

    /// The series, its library, and its books with their media, soft-deleted ones included.
    async fn load_series(
        &self,
        id: &SeriesId,
    ) -> Result<Option<(Series, Library, Vec<(Book, Media)>)>, MetadataError> {
        let mut tx = self.db.begin().await?;
        let Some(series) = SeriesDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            return Ok(None);
        };
        let library = LibraryDao::find_by_id(&mut tx, &series.library_id)
            .await?
            .expect("A series' library exists (foreign key)");
        let mut books = Vec::new();
        for book in BookDao::find_all_by_series(&mut tx, id).await? {
            let media = MediaDao::find_by_id(&mut tx, &book.id)
                .await?
                .ok_or_else(|| MetadataError::MissingMedia(book.id.clone()))?;
            books.push((book, media));
        }
        tx.rollback().await?;
        Ok(Some((series, library, books)))
    }
}

/// The patches of the providers that read series metadata from each book, aggregated per
/// provider, and the distinct names of the collections they found.
async fn from_books(
    library: &Library,
    books: &[(Book, Media)],
) -> (Vec<SeriesMetadataPatch>, Vec<String>) {
    let mut patches = Vec::new();
    let mut collections: Vec<String> = Vec::new();
    for provider in SERIES_FROM_BOOK_PROVIDERS {
        let name = provider.name();
        let to_series = provider.handles(library, Target::Series);
        let to_collections = provider.handles(library, Target::Collection);
        if !to_series && !to_collections {
            info!(
                provider = name,
                "Library is not set to import series or collection metadata for this provider, \
                 skipping"
            );
            continue;
        }
        debug!(provider = name, "Provider");
        let append_volume = library.import_comic_info_series_append_volume;
        let mut found = Vec::new();
        for (book, media) in books {
            found.extend(provider.series_patch(book, media, append_volume).await);
        }
        if to_series {
            patches.push(series_patch::aggregate(&found));
        }
        if to_collections {
            for name in found.iter().flat_map(|patch| &patch.collections) {
                if !collections.contains(name) {
                    collections.push(name.clone());
                }
            }
        }
    }
    (patches, collections)
}
