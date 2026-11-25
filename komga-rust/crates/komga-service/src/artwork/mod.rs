//! Book, series, read list and collection thumbnails: Kotlin
//! `BookLifecycle.{generateThumbnailAndPersist,addThumbnailForBook,getThumbnail,getThumbnailBytes}`,
//! `SeriesLifecycle.getThumbnailBytes`, `ReadListLifecycle` and `SeriesCollectionLifecycle`
//! posters, `BookAnalyzer.{generateThumbnail,getPoster}` and `LocalArtworkLifecycle`.
//!
//! Thumbnails are served from the database alone, so serving one never touches the library's
//! storage. Only sidecar rows left by the Kotlin server without their bytes are read once from
//! their file.

mod book_poster;
mod book_thumbnails;
mod error;
mod generation;
pub(crate) mod house_keeping;
mod local_artwork;
mod local_files;
mod series;
mod set_thumbnails;
mod thumbnail_generator;
mod upload;

use std::io;

use komga_db::{
    BookDao, Database, MediaDao, ThumbnailBookDao, ThumbnailContent, ThumbnailSeriesDao,
};
use komga_scheduler::{Task, TaskQueue};
use komga_types::{
    Book, BookId, FileUrl, KomgaUser, MediaStatus, SeriesId, ThumbnailId, ThumbnailType,
};
use tracing::{info, warn};

pub use error::ArtworkError;

use crate::events::EventBus;
use crate::support::access_policy;
use error::{check, generator_error};

#[derive(Clone, Copy)]
enum ThumbnailOwner<'a> {
    Book(&'a BookId),
    Series(&'a SeriesId),
}

#[derive(Clone)]
pub struct ArtworkService {
    db: Database,
    tasks: TaskQueue,
    events: EventBus,
}

impl ArtworkService {
    #[must_use]
    pub fn new(db: Database, tasks: TaskQueue, events: EventBus) -> Self {
        Self { db, tasks, events }
    }

    /// The book's selected thumbnail.
    pub async fn book_thumbnail(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<Vec<u8>, ArtworkError> {
        self.visible_book(user, id).await?;
        self.selected_book_thumbnail(id)
            .await?
            .ok_or(ArtworkError::NotFound)
    }

    /// The book, unless `user` may not see it.
    async fn visible_book(&self, user: &KomgaUser, id: &BookId) -> Result<Book, ArtworkError> {
        let mut tx = self.db.begin().await?;
        let book = BookDao::find_by_id(&mut tx, id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        if !access_policy::can_access_book(&mut tx, user, &book).await? {
            return Err(ArtworkError::Restricted);
        }
        tx.rollback().await?;
        Ok(book)
    }

    /// `BookLifecycle.getThumbnailBytes`.
    async fn selected_book_thumbnail(&self, id: &BookId) -> Result<Option<Vec<u8>>, ArtworkError> {
        Ok(self
            .selected_book_thumbnail_of_kind(id)
            .await?
            .map(|(_, bytes)| bytes))
    }

    /// The selected thumbnail and its kind. A ready book without a thumbnail gets its generation
    /// queued, unless that is queued, running or failed already, and has none meanwhile.
    async fn selected_book_thumbnail_of_kind(
        &self,
        id: &BookId,
    ) -> Result<Option<(ThumbnailType, Vec<u8>)>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        let mut selected = ThumbnailBookDao::find_selected(&mut tx, id).await?;
        tx.rollback().await?;
        loop {
            let (stored, ready) = match selected.take() {
                Some(selected) if selected.is_stored() => (Some(selected), false),
                _ => self.house_keeping(id).await?,
            };
            let Some(stored) = stored else {
                if ready
                    && self
                        .tasks
                        .submit_if_absent(&Task::generate_book_thumbnail(id.clone()))
                        .await?
                {
                    info!(book = %id, "Book has no thumbnail, generating one");
                }
                return Ok(None);
            };
            match (stored.thumbnail, stored.url) {
                (Some(bytes), _) => return Ok(Some((stored.kind, bytes))),
                (None, Some(url)) => {
                    let owner = ThumbnailOwner::Book(id);
                    if let Some(bytes) = self.cache_kotlin_sidecar(owner, &stored.id, &url).await? {
                        return Ok(Some((stored.kind, bytes)));
                    }
                }
                (None, None) => unreachable!("Stored thumbnails have bytes or a URL"),
            }
        }
    }

    /// Runs the house keeping, returning the selected thumbnail and, without one, whether the
    /// book is ready for a generation.
    async fn house_keeping(
        &self,
        id: &BookId,
    ) -> Result<(Option<ThumbnailContent>, bool), ArtworkError> {
        let mut tx = self.db.begin_immediate().await?;
        house_keeping::book(&mut tx, id).await?;
        let selected = ThumbnailBookDao::find_selected(&mut tx, id).await?;
        let mut ready = false;
        if selected.is_none() {
            let media = MediaDao::find_by_id(&mut tx, id).await?;
            ready = media.is_some_and(|media| media.status == MediaStatus::Ready);
        }
        tx.commit().await?;
        Ok((selected, ready))
    }

    /// Stores the bytes of a sidecar row the Kotlin server left with only its URL, which it read
    /// on every request. A missing file drops the row, as Kotlin's `exists` check does, and gives
    /// `None`.
    async fn cache_kotlin_sidecar(
        &self,
        owner: ThumbnailOwner<'_>,
        thumbnail_id: &ThumbnailId,
        url: &FileUrl,
    ) -> Result<Option<Vec<u8>>, ArtworkError> {
        let path = url
            .to_path()
            .ok_or_else(|| ArtworkError::InvalidUrl(url.clone()))?;
        let read = local_files::read_bounded(&path).await;
        let mut tx = self.db.begin_immediate().await?;
        let bytes = match read {
            Ok(bytes) => {
                match owner {
                    ThumbnailOwner::Book(_) => {
                        ThumbnailBookDao::fill_thumbnail(&mut tx, thumbnail_id, &bytes).await?;
                    }
                    ThumbnailOwner::Series(_) => {
                        ThumbnailSeriesDao::fill_thumbnail(&mut tx, thumbnail_id, &bytes).await?;
                    }
                }
                Some(bytes)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match owner {
                    ThumbnailOwner::Book(book_id) => {
                        warn!(book = %book_id, thumbnail = %thumbnail_id, "Thumbnail doesn't exist, removing entry");
                        ThumbnailBookDao::delete(&mut tx, thumbnail_id).await?;
                    }
                    ThumbnailOwner::Series(series_id) => {
                        warn!(series = %series_id, thumbnail = %thumbnail_id, "Thumbnail doesn't exist, removing entry");
                        ThumbnailSeriesDao::delete(&mut tx, thumbnail_id).await?;
                    }
                }
                None
            }
            Err(error) => return Err(error.into()),
        };
        tx.commit().await?;
        Ok(bytes)
    }
}
