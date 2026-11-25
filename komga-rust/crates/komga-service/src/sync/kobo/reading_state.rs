//! Kotlin `KoboController.getState` and `updateState`: a book's reading state on a device.

use jiff::Timestamp;
use jiff::civil::DateTime;
use komga_db::{BookDao, DbTransaction, KoboDao, MediaDao};
use komga_types::{BookId, KoboReadProgress, KomgaUser, R2Device, R2Locator, R2Progression};
use tracing::error;

use super::{KoboError, KoboReply, KoboSyncService};
use crate::support::access_policy;

/// What the device reports about a book.
#[derive(Debug, Clone, PartialEq)]
pub struct KoboReadingUpdate {
    pub modified: Timestamp,
    /// `None` for a finished book: Kobo then points at its first resource, so the book's last
    /// position is recorded instead.
    pub locator: Option<R2Locator>,
}

/// A book's reading state: the user's progress, or the book's creation for an unread book.
#[derive(Debug, Clone, PartialEq)]
pub struct KoboReadingState {
    pub book_created: DateTime,
    pub progress: Option<KoboReadProgress>,
}

impl KoboSyncService {
    /// `GET /kobo/{key}/v1/library/{bookId}/state`.
    pub async fn reading_state(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<KoboReply<KoboReadingState>, KoboError> {
        let mut tx = self.db.begin().await?;
        let Some(book_created) = self.visible_book(&mut tx, user, id).await? else {
            return Ok(KoboReply::Store);
        };
        let progress = KoboDao::find_read_progress(&mut tx, &user.id, std::slice::from_ref(id))
            .await?
            .pop();
        tx.rollback().await?;
        Ok(KoboReply::Local(KoboReadingState {
            book_created,
            progress,
        }))
    }

    /// `PUT /kobo/{key}/v1/library/{bookId}/state`: `None` for an update Kotlin rejects as a bad
    /// request. Whether the progress was recorded; a failure is logged, as Kobo is told only
    /// that it failed.
    pub async fn update_reading_state(
        &self,
        user: &KomgaUser,
        id: &BookId,
        device: R2Device,
        update: Option<KoboReadingUpdate>,
    ) -> Result<KoboReply<bool>, KoboError> {
        let mut tx = self.db.begin().await?;
        if self.visible_book(&mut tx, user, id).await?.is_none() {
            return Ok(KoboReply::Store);
        }
        let update = update.ok_or(KoboError::BadRequest)?;
        let locator = match update.locator {
            Some(locator) => locator,
            None => {
                let last = MediaDao::find_extension(&mut tx, id)
                    .await?
                    .and_then(|extension| extension.positions.last().cloned());
                let Some(locator) = last else {
                    error!(book = %id, "No EPUB positions for a finished book, it should be analyzed again");
                    return Ok(KoboReply::Local(false));
                };
                locator
            }
        };
        let progression = R2Progression {
            modified: update.modified,
            device,
            locator,
        };
        tx.rollback().await?;
        let recorded = self
            .read_progress
            .mark_progression(user, id, progression)
            .await
            .inspect_err(|error| error!(book = %id, %error, "Could not update progression"))
            .is_ok();
        Ok(KoboReply::Local(recorded))
    }

    /// The book's creation if the user sees it; `None` for an unknown book the store answers.
    async fn visible_book(
        &self,
        tx: &mut DbTransaction,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<Option<DateTime>, KoboError> {
        let book = BookDao::find_by_id(tx, id).await?;
        let created = KoboDao::find_book_created(tx, id).await?;
        let (Some(book), Some(created)) = (book, created) else {
            return if self.settings().await?.proxy {
                Ok(None)
            } else {
                Err(KoboError::NotFound)
            };
        };
        if access_policy::can_access_book(tx, user, &book).await? {
            Ok(Some(created))
        } else {
            Err(KoboError::Restricted)
        }
    }
}
