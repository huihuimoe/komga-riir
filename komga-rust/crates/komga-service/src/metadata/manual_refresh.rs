//! Kotlin `BookController.bookRefreshMetadata`, `SeriesController.seriesRefreshMetadata`, and
//! `LibraryController.libraryRefreshMetadata`: an admin's request to refresh every metadata
//! capability and the local artwork, ahead of default tasks. Soft-deleted books and series are
//! included, as in Kotlin.

use komga_db::{BookDao, SeriesDao};
use komga_scheduler::Task;
use komga_types::{Book, BookId, BookMetadataPatchCapability, KomgaUser, LibraryId, SeriesId};

use super::{MetadataError, MetadataService};

impl MetadataService {
    pub async fn submit_book_refresh(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<(), MetadataError> {
        require_admin(user)?;
        let mut tx = self.db.begin().await?;
        let book = BookDao::find_by_id(&mut tx, id).await?;
        tx.rollback().await?;
        let book = book.ok_or(MetadataError::NotFound)?;
        self.tasks.submit_all(&book_refreshes(&[book])).await?;
        Ok(())
    }

    /// Kotlin does not check that the series exists: an unknown one queues only its artwork
    /// refresh, which finds nothing.
    pub async fn submit_series_refresh(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
    ) -> Result<(), MetadataError> {
        require_admin(user)?;
        let mut tx = self.db.begin().await?;
        let books = BookDao::find_all_by_series(&mut tx, id).await?;
        tx.rollback().await?;
        let mut tasks = book_refreshes(&books);
        tasks.push(Task::refresh_series_local_artwork(id.clone()).high());
        self.tasks.submit_all(&tasks).await?;
        Ok(())
    }

    /// Kotlin does not check that the library exists: an unknown one queues nothing.
    pub async fn submit_library_refresh(
        &self,
        user: &KomgaUser,
        id: &LibraryId,
    ) -> Result<(), MetadataError> {
        require_admin(user)?;
        let mut tx = self.db.begin().await?;
        let books = BookDao::find_all_by_library(&mut tx, id).await?;
        let series = SeriesDao::find_all_by_library(&mut tx, id).await?;
        tx.rollback().await?;
        let mut tasks = book_refreshes(&books);
        tasks.extend(
            series
                .into_iter()
                .map(|series| Task::refresh_series_local_artwork(series.id).high()),
        );
        self.tasks.submit_all(&tasks).await?;
        Ok(())
    }
}

pub(super) fn require_admin(user: &KomgaUser) -> Result<(), MetadataError> {
    if user.is_admin() {
        Ok(())
    } else {
        Err(MetadataError::Forbidden)
    }
}

/// Every book's metadata refresh, then every book's local artwork refresh, as Kotlin submits them.
fn book_refreshes(books: &[Book]) -> Vec<Task> {
    let metadata = books.iter().map(|book| {
        Task::refresh_book_metadata(
            book.id.clone(),
            book.series_id.clone(),
            BookMetadataPatchCapability::ALL.to_vec(),
        )
        .high()
    });
    let artwork = books
        .iter()
        .map(|book| Task::refresh_book_local_artwork(book.id.clone()).high());
    metadata.chain(artwork).collect()
}
