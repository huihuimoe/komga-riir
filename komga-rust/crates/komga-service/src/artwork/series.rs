//! Kotlin `SeriesController` poster routes and
//! `SeriesLifecycle.{getSelectedThumbnail,getThumbnailBytes,getThumbnailBytesByThumbnailId,
//! addThumbnailForSeries,deleteThumbnailForSeries}`.
//!
//! Unlike books, adding an unselected series poster or deleting one runs no house keeping: the
//! next request for the series thumbnail selects another.

use komga_db::{
    BookDao, DbTransaction, LibraryDao, SeriesDao, ThumbnailContent, ThumbnailSeriesDao,
};
use komga_types::{
    BookId, DomainEvent, KomgaUser, Series, SeriesCover, SeriesId, SeriesRef, ThumbnailDetails,
    ThumbnailId, ThumbnailRef, ThumbnailType,
};
use tracing::info;

use super::{ArtworkError, ArtworkService, ThumbnailOwner, check, house_keeping, upload};
use crate::support::access_policy;

impl ArtworkService {
    /// The series' selected thumbnail or, without one, the thumbnail of the book its library's
    /// `seriesCover` picks.
    pub async fn series_thumbnail(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
    ) -> Result<Vec<u8>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        let series = SeriesDao::find_by_id(&mut tx, id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        if !access_policy::can_access_series(&mut tx, user, &series).await? {
            return Err(ArtworkError::Restricted);
        }
        tx.rollback().await?;
        self.selected_series_thumbnail(user, &series)
            .await?
            .ok_or(ArtworkError::NotFound)
    }

    /// `SeriesLifecycle.getThumbnailBytes` for `user`, who may see `series`.
    pub(super) async fn selected_series_thumbnail(
        &self,
        user: &KomgaUser,
        series: &Series,
    ) -> Result<Option<Vec<u8>>, ArtworkError> {
        let id = &series.id;
        let mut tx = self.db.begin().await?;
        let mut selected = ThumbnailSeriesDao::find_selected(&mut tx, id).await?;
        tx.rollback().await?;
        loop {
            let stored = match selected.take() {
                Some(selected) if selected.is_stored() => Some(selected),
                _ => self.series_house_keeping(id).await?,
            };
            let Some(stored) = stored else {
                break;
            };
            match (stored.thumbnail, stored.url) {
                (Some(bytes), _) => return Ok(Some(bytes)),
                (None, Some(url)) => {
                    let owner = ThumbnailOwner::Series(id);
                    if let Some(bytes) = self.cache_kotlin_sidecar(owner, &stored.id, &url).await? {
                        return Ok(Some(bytes));
                    }
                }
                (None, None) => unreachable!("Stored thumbnails have bytes or a URL"),
            }
        }
        match self.cover_book(user, series).await? {
            Some(book_id) => self.selected_book_thumbnail(&book_id).await,
            None => Ok(None),
        }
    }

    async fn series_house_keeping(
        &self,
        id: &SeriesId,
    ) -> Result<Option<ThumbnailContent>, ArtworkError> {
        let mut tx = self.db.begin_immediate().await?;
        house_keeping::series(&mut tx, id).await?;
        let selected = ThumbnailSeriesDao::find_selected(&mut tx, id).await?;
        tx.commit().await?;
        Ok(selected)
    }

    /// The book whose thumbnail stands for the series. A library deleted meanwhile has none.
    async fn cover_book(
        &self,
        user: &KomgaUser,
        series: &Series,
    ) -> Result<Option<BookId>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        let Some(library) = LibraryDao::find_by_id(&mut tx, &series.library_id).await? else {
            return Ok(None);
        };
        let id = &series.id;
        let book_id = match library.series_cover {
            SeriesCover::First => BookDao::find_first_id_in_series(&mut tx, id).await?,
            SeriesCover::FirstUnreadOrFirst => {
                match BookDao::find_first_unread_id_in_series(&mut tx, id, &user.id).await? {
                    Some(book_id) => Some(book_id),
                    None => BookDao::find_first_id_in_series(&mut tx, id).await?,
                }
            }
            SeriesCover::FirstUnreadOrLast => {
                match BookDao::find_first_unread_id_in_series(&mut tx, id, &user.id).await? {
                    Some(book_id) => Some(book_id),
                    None => BookDao::find_last_id_in_series(&mut tx, id).await?,
                }
            }
            SeriesCover::Last => BookDao::find_last_id_in_series(&mut tx, id).await?,
        };
        tx.rollback().await?;
        Ok(book_id)
    }

    /// `GET /api/v1/series/{seriesId}/thumbnails`.
    pub async fn series_thumbnails(
        &self,
        user: &KomgaUser,
        series_id: &SeriesId,
    ) -> Result<Vec<ThumbnailDetails>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        check(access_policy::check_series_id(&mut tx, user, series_id).await?)?;
        let thumbnails = ThumbnailSeriesDao::find_all_details_by_series(&mut tx, series_id).await?;
        tx.rollback().await?;
        Ok(thumbnails)
    }

    /// `GET /api/v1/series/{seriesId}/thumbnails/{thumbnailId}`: the series and the thumbnail's
    /// own series are checked separately, and need not be the same.
    pub async fn series_thumbnail_by_id(
        &self,
        user: &KomgaUser,
        series_id: &SeriesId,
        id: &ThumbnailId,
    ) -> Result<Vec<u8>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        check(access_policy::check_series_id(&mut tx, user, series_id).await?)?;
        let (owner, _) = ThumbnailSeriesDao::find_owner(&mut tx, id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        check(access_policy::check_series_id(&mut tx, user, &owner).await?)?;
        let content = ThumbnailSeriesDao::find_content(&mut tx, id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        tx.rollback().await?;
        match (content.thumbnail, content.url) {
            (Some(bytes), _) => Ok(bytes),
            (None, Some(url)) => self
                .cache_kotlin_sidecar(ThumbnailOwner::Series(&owner), id, &url)
                .await?
                .ok_or(ArtworkError::NotFound),
            (None, None) => Err(ArtworkError::NotFound),
        }
    }

    /// `POST /api/v1/series/{seriesId}/thumbnails`: `addThumbnailForSeries` of an uploaded image
    /// with `YES` or `NO`; oneshots take their book's posters instead.
    pub async fn add_series_thumbnail(
        &self,
        series_id: &SeriesId,
        bytes: &[u8],
        selected: bool,
    ) -> Result<ThumbnailDetails, ArtworkError> {
        let mut read_tx = self.db.begin().await?;
        uploadable_series(&mut read_tx, series_id).await?;
        read_tx.rollback().await?;
        let details = upload::details(bytes, selected).await?;
        let mut tx = self.db.begin_immediate().await?;
        let series = uploadable_series(&mut tx, series_id).await?;
        ThumbnailSeriesDao::insert(&mut tx, series_id, &upload::row(&details, bytes)).await?;
        if selected {
            ThumbnailSeriesDao::mark_selected(&mut tx, series_id, &details.id).await?;
        }
        tx.record(DomainEvent::ThumbnailSeriesAdded(ThumbnailRef {
            owner: SeriesRef::from(&series),
            selected,
        }));
        self.events.commit(tx).await?;
        info!(series = %series_id, thumbnail = %details.id, "Added uploaded thumbnail");
        Ok(details)
    }

    /// `PUT /api/v1/series/{seriesId}/thumbnails/{thumbnailId}/selected`.
    pub async fn select_series_thumbnail(
        &self,
        series_id: &SeriesId,
        id: &ThumbnailId,
    ) -> Result<(), ArtworkError> {
        let mut tx = self.db.begin_immediate().await?;
        let (owner, _) = owned_kind(&mut tx, series_id, id).await?;
        ThumbnailSeriesDao::mark_selected(&mut tx, series_id, id).await?;
        tx.record(DomainEvent::ThumbnailSeriesAdded(ThumbnailRef {
            owner,
            selected: true,
        }));
        self.events.commit(tx).await?;
        Ok(())
    }

    /// `DELETE /api/v1/series/{seriesId}/thumbnails/{thumbnailId}`: only uploaded thumbnails.
    pub async fn delete_series_thumbnail(
        &self,
        series_id: &SeriesId,
        id: &ThumbnailId,
    ) -> Result<(), ArtworkError> {
        let mut tx = self.db.begin_immediate().await?;
        let (owner, kind) = owned_kind(&mut tx, series_id, id).await?;
        if kind != ThumbnailType::UserUploaded {
            return Err(ArtworkError::NotUploaded);
        }
        let selected = ThumbnailSeriesDao::delete(&mut tx, id).await?;
        tx.record(DomainEvent::ThumbnailSeriesDeleted(ThumbnailRef {
            owner,
            selected,
        }));
        self.events.commit(tx).await?;
        info!(series = %series_id, thumbnail = %id, "Deleted uploaded thumbnail");
        Ok(())
    }
}

/// An existing non-oneshot series eligible for uploaded series posters.
async fn uploadable_series(
    tx: &mut DbTransaction,
    series_id: &SeriesId,
) -> Result<Series, ArtworkError> {
    let series = SeriesDao::find_by_id(tx, series_id)
        .await?
        .ok_or(ArtworkError::NotFound)?;
    if series.oneshot {
        return Err(ArtworkError::Oneshot);
    }
    Ok(series)
}

/// An existing series and the type of its thumbnail, which must be that series'.
async fn owned_kind(
    tx: &mut DbTransaction,
    series_id: &SeriesId,
    id: &ThumbnailId,
) -> Result<(SeriesRef, ThumbnailType), ArtworkError> {
    let series = SeriesDao::find_by_id(tx, series_id)
        .await?
        .ok_or(ArtworkError::NotFound)?;
    let (owner, kind) = ThumbnailSeriesDao::find_owner(tx, id)
        .await?
        .ok_or(ArtworkError::NotFound)?;
    if owner != *series_id {
        return Err(ArtworkError::OtherOwner);
    }
    Ok((SeriesRef::from(&series), kind))
}
