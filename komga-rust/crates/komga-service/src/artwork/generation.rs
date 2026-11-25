use komga_db::{BookDao, DbError, DbTransaction, MediaDao, NewThumbnail, ThumbnailBookDao};
use komga_scheduler::Task;
use komga_types::{
    BookId, BookRef, DomainEvent, MediaStatus, ThumbnailId, ThumbnailRef, ThumbnailType,
};
use tokio::task;
use tracing::{info, warn};

use super::error::generator_error;
use super::{ArtworkError, ArtworkService, house_keeping, thumbnail_generator};
use crate::settings::ServerSettings;

impl ArtworkService {
    /// `PUT /api/v1/books/thumbnails`: queues the search for books to regenerate.
    pub async fn submit_regeneration(
        &self,
        for_bigger_result_only: bool,
    ) -> Result<(), ArtworkError> {
        self.tasks
            .submit(&Task::find_book_thumbnails_to_regenerate(for_bigger_result_only).lowest())
            .await?;
        Ok(())
    }

    /// `BookLifecycle.findBookThumbnailsToRegenerate`: with `for_bigger_result_only`, the books
    /// whose generated thumbnail is smaller than the configured size on both edges; otherwise
    /// every book that is not soft-deleted.
    pub(crate) async fn find_thumbnails_to_regenerate(
        &self,
        for_bigger_result_only: bool,
        priority: i32,
    ) -> Result<(), ArtworkError> {
        let size = ServerSettings::load(&self.db).await?.thumbnail_size;
        let mut tx = self.db.begin().await?;
        let book_ids = if for_bigger_result_only {
            ThumbnailBookDao::find_book_ids_by_type_smaller_than(
                &mut tx,
                ThumbnailType::Generated,
                size.max_edge(),
            )
            .await?
        } else {
            BookDao::find_all_not_deleted_ids(&mut tx).await?
        };
        tx.rollback().await?;
        let tasks: Vec<Task> = book_ids
            .into_iter()
            .map(|book_id| Task::generate_book_thumbnail(book_id).with_priority(priority))
            .collect();
        self.tasks.submit_all(&tasks).await?;
        Ok(())
    }

    /// Makes the book's generated thumbnail from its cover at the configured size, and selects it
    /// unless another kind is selected. The result is dropped when the media changed meanwhile:
    /// the analysis that changed it queues a new generation.
    pub(crate) async fn generate_and_persist(&self, id: &BookId) -> Result<(), ArtworkError> {
        let mut tx = self.db.begin().await?;
        let Some(book) = BookDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            warn!(book = %id, "Cannot generate thumbnail: Book does not exist");
            return Ok(());
        };
        let media = MediaDao::find_by_id(&mut tx, id).await?;
        let version = MediaDao::find_last_modified_date(&mut tx, id).await?;
        tx.rollback().await?;
        let (Some(media), Some(version)) = (media, version) else {
            return Err(ArtworkError::MissingMedia(id.clone()));
        };
        if media.status != MediaStatus::Ready {
            warn!(book = %id, "Book media is not ready, cannot generate thumbnail");
            return Ok(());
        }
        info!(book = %id, url = %book.url, "Generate thumbnail and persist for book");
        let size = ServerSettings::load(&self.db).await?.thumbnail_size;
        let path = book
            .url
            .to_path()
            .ok_or_else(|| ArtworkError::InvalidUrl(book.url.clone()))?;
        let generated =
            task::spawn_blocking(move || thumbnail_generator::generate(&path, &media, size))
                .await?
                .map_err(|error| generator_error(id, error))?;

        let mut tx = self.db.begin_immediate().await?;
        if MediaDao::find_last_modified_date(&mut tx, id).await? != Some(version) {
            tx.rollback().await?;
            warn!(book = %id, "Book media changed during thumbnail generation, dropping it");
            return Ok(());
        }
        let selected = add_generated(&mut tx, id, &generated).await?;
        tx.record(DomainEvent::ThumbnailBookAdded(ThumbnailRef {
            owner: BookRef::from(&book),
            selected,
        }));
        self.events.commit(tx).await?;
        Ok(())
    }
}

/// `addThumbnailForBook` of a generated thumbnail with `IF_NONE_OR_GENERATED`: it replaces the
/// previous generated one and is selected unless a thumbnail of another kind is. Returns whether
/// it is selected.
async fn add_generated(
    tx: &mut DbTransaction,
    book_id: &BookId,
    generated: &thumbnail_generator::Generated,
) -> Result<bool, DbError> {
    ThumbnailBookDao::delete_by_book_and_type(tx, book_id, ThumbnailType::Generated).await?;
    let id = ThumbnailId::generate();
    ThumbnailBookDao::insert(
        tx,
        book_id,
        &NewThumbnail {
            id: &id,
            thumbnail: Some(&generated.bytes),
            url: None,
            kind: ThumbnailType::Generated,
            media_type: generated.media_type,
            file_size: i64::try_from(generated.bytes.len()).expect("A thumbnail fits in i64"),
            dimension: generated.dimension,
        },
    )
    .await?;
    let selected = ThumbnailBookDao::find_selected(tx, book_id).await?;
    let select = selected.is_none_or(|selected| selected.kind == ThumbnailType::Generated);
    if select {
        ThumbnailBookDao::mark_selected(tx, book_id, &id).await?;
    } else {
        house_keeping::book(tx, book_id).await?;
    }
    Ok(select)
}
