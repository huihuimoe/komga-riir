//! Kotlin `KoboController`'s rules, apart from the Kobo protocol encoding.

use komga_db::{Database, DbError, KoboDao};
use komga_image::{ImageError, ImageType};
use komga_types::{BookId, KoboBook, KomgaUser, ThumbnailId};
use thiserror::Error;

use crate::settings::SettingsError;
use crate::support::access_policy::{self, Access};
use crate::support::blocking::{self, BlockingError};
use crate::{ArtworkError, ArtworkService, ReadProgressService, ServerSettings, SyncPointService};

mod library;
mod reading_state;

pub use library::{KoboChanges, KoboSync};
pub use reading_state::KoboReadingUpdate;

#[derive(Debug, Error)]
pub enum KoboError {
    #[error("Not found")]
    NotFound,
    /// `ContentRestrictionChecker` denied the book.
    #[error("Content restricted")]
    Restricted,
    #[error("Bad request")]
    BadRequest,
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Artwork(ArtworkError),
    #[error(transparent)]
    Blocking(#[from] BlockingError),
    #[error("Cannot convert the cover: {0}")]
    Image(#[from] ImageError),
}

/// Where a request is answered from.
#[derive(Debug, PartialEq)]
pub enum KoboReply<T> {
    Local(T),
    /// Komga does not know the item and the Kobo store proxy is on: the store answers.
    Store,
}

/// The server settings Kobo requests depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KoboSettings {
    pub proxy: bool,
    /// The port image and download URLs use when no forwarded headers say otherwise.
    pub port: Option<i32>,
}

#[derive(Clone)]
pub struct KoboSyncService {
    db: Database,
    artwork: ArtworkService,
    sync_points: SyncPointService,
    read_progress: ReadProgressService,
    /// Items a sync page holds at most.
    sync_item_limit: u32,
}

impl KoboSyncService {
    #[must_use]
    pub fn new(
        db: Database,
        artwork: ArtworkService,
        read_progress: ReadProgressService,
        sync_item_limit: u32,
    ) -> Self {
        Self {
            sync_points: SyncPointService::new(db.clone()),
            db,
            artwork,
            read_progress,
            sync_item_limit,
        }
    }

    pub async fn settings(&self) -> Result<KoboSettings, KoboError> {
        let settings = ServerSettings::load(&self.db).await?;
        Ok(KoboSettings {
            proxy: settings.kobo_proxy,
            port: settings.kobo_port,
        })
    }

    /// `GET /kobo/{key}/v1/library/{bookId}/metadata`. A user who sees every library gets no
    /// book for an unknown ID, as in Kotlin.
    pub async fn book_metadata(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<KoboReply<Vec<KoboBook>>, KoboError> {
        let mut tx = self.db.begin().await?;
        let books = KoboDao::find_books(&mut tx, std::slice::from_ref(id)).await?;
        if books.is_empty() && self.settings().await?.proxy {
            return Ok(KoboReply::Store);
        }
        match access_policy::check_book_id(&mut tx, user, id).await? {
            Access::Allowed => {}
            Access::Denied => return Err(KoboError::Restricted),
            Access::Unknown => return Err(KoboError::NotFound),
        }
        tx.rollback().await?;
        Ok(KoboReply::Local(books))
    }

    /// `GET /kobo/{key}/v1/books/{thumbnailId}/thumbnail/…`: the thumbnail as JPEG, whatever
    /// size is asked.
    pub async fn cover(
        &self,
        user: &KomgaUser,
        id: &ThumbnailId,
    ) -> Result<KoboReply<Vec<u8>>, KoboError> {
        let (bytes, media_type) = match self.artwork.book_thumbnail_by_id_only(user, id).await {
            Ok(thumbnail) => thumbnail,
            Err(ArtworkError::NotFound) if self.settings().await?.proxy => {
                return Ok(KoboReply::Store);
            }
            Err(ArtworkError::NotFound) => return Err(KoboError::NotFound),
            Err(ArtworkError::Restricted) => return Err(KoboError::Restricted),
            Err(error) => return Err(KoboError::Artwork(error)),
        };
        if media_type == ImageType::Jpeg.media_type() {
            return Ok(KoboReply::Local(bytes));
        }
        let jpeg = blocking::run(move || komga_image::convert(&bytes, ImageType::Jpeg)).await??;
        Ok(KoboReply::Local(jpeg))
    }
}
