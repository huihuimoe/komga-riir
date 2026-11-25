//! Kotlin `SeriesLifecycle.markReadProgressCompleted` and `deleteReadProgress`.

use jiff::Timestamp;
use komga_db::ReadProgressDao;
use komga_types::{DomainEvent, KomgaUser, SeriesId, SeriesProgressRef};

use super::{ReadProgressError, ReadProgressService, record_books};
use crate::support::access_policy::{self, Access};

impl ReadProgressService {
    /// Marks every book of the series the user has not completed as read now.
    pub async fn mark_series_completed(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
    ) -> Result<(), ReadProgressError> {
        let mut tx = self.db.begin_immediate().await?;
        check(access_policy::check_series_id(&mut tx, user, id).await?)?;
        let completed =
            ReadProgressDao::complete_series(&mut tx, id, None, &user.id, Timestamp::now()).await?;
        record_books(&mut tx, completed, user, DomainEvent::ReadProgressChanged);
        tx.record(DomainEvent::ReadProgressSeriesChanged(series_progress(
            id, user,
        )));
        self.events.commit(tx).await?;
        Ok(())
    }

    /// Makes every book of the series unread for the user.
    pub async fn delete_series(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
    ) -> Result<(), ReadProgressError> {
        let mut tx = self.db.begin_immediate().await?;
        check(access_policy::check_series_id(&mut tx, user, id).await?)?;
        let deleted = ReadProgressDao::delete_series(&mut tx, id, &user.id).await?;
        record_books(&mut tx, deleted, user, DomainEvent::ReadProgressDeleted);
        tx.record(DomainEvent::ReadProgressSeriesDeleted(series_progress(
            id, user,
        )));
        self.events.commit(tx).await?;
        Ok(())
    }
}

fn series_progress(id: &SeriesId, user: &KomgaUser) -> SeriesProgressRef {
    SeriesProgressRef {
        series_id: id.clone(),
        user_id: user.id.clone(),
    }
}

/// A user who sees every library is not checked against the series, so for them an unknown series
/// changes nothing, as in Kotlin.
pub(super) fn check(access: Access) -> Result<(), ReadProgressError> {
    match access {
        Access::Allowed => Ok(()),
        Access::Denied => Err(ReadProgressError::Restricted),
        Access::Unknown => Err(ReadProgressError::NotFound),
    }
}
