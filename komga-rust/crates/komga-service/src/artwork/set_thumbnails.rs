//! Kotlin `ReadListController` and `SeriesCollectionController` poster routes, and the
//! `getThumbnailBytes`, `addThumbnail`, `markSelectedThumbnail` and `deleteThumbnail` of
//! `ReadListLifecycle` and `SeriesCollectionLifecycle`.

use komga_db::{
    DbTransaction, PosterSet, ReadListDao, SeriesCollectionDao, SeriesDao, ThumbnailSetDao,
};
use komga_image::ImageType;
use komga_types::{
    CollectionId, ContentScope, DomainEvent, KomgaUser, ReadListId, SetId, ThumbnailDetails,
    ThumbnailId, ThumbnailRef,
};
use tracing::info;

use super::{ArtworkError, ArtworkService, house_keeping, upload};
use crate::settings::ServerSettings;
use crate::support::blocking;

/// Members, then the quadrants they fill, of a poster made from member thumbnails.
const MOSAIC_IMAGES: usize = 4;

impl ArtworkService {
    /// `GET /api/v1/readlists/{id}/thumbnail`: the selected poster or, without one, a mosaic of
    /// the first four books `user` sees. Kotlin never answers for a read list without books; it
    /// gets a black poster here.
    pub async fn read_list_thumbnail(
        &self,
        user: &KomgaUser,
        id: &ReadListId,
    ) -> Result<Vec<u8>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        let scope = ContentScope::for_user(user);
        let read_list = ReadListDao::find_by_id(&mut tx, id, Some(&scope))
            .await?
            .ok_or(ArtworkError::NotFound)?;
        let selected = ThumbnailSetDao::find_selected_thumbnail(&mut tx, id).await?;
        tx.rollback().await?;
        if let Some(bytes) = selected {
            return Ok(bytes);
        }
        let mut thumbnails = Vec::new();
        for book_id in read_list.book_ids.iter().take(MOSAIC_IMAGES) {
            thumbnails.push(self.selected_book_thumbnail(book_id).await?);
        }
        self.mosaic(thumbnails).await
    }

    /// `GET /api/v1/collections/{id}/thumbnail`: the selected poster or, without one, a mosaic of
    /// the thumbnails of the first four series `user` sees, as `user` sees them; black without
    /// series, where Kotlin never answers.
    pub async fn collection_thumbnail(
        &self,
        user: &KomgaUser,
        id: &CollectionId,
    ) -> Result<Vec<u8>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        let scope = ContentScope::for_user(user);
        let collection = SeriesCollectionDao::find_by_id(&mut tx, id, Some(&scope))
            .await?
            .ok_or(ArtworkError::NotFound)?;
        if let Some(bytes) = ThumbnailSetDao::find_selected_thumbnail(&mut tx, id).await? {
            tx.rollback().await?;
            return Ok(bytes);
        }
        let mut series = Vec::new();
        for series_id in collection.series_ids.iter().take(MOSAIC_IMAGES) {
            series.extend(SeriesDao::find_by_id(&mut tx, series_id).await?);
        }
        tx.rollback().await?;
        let mut thumbnails = Vec::new();
        for series in &series {
            thumbnails.push(self.selected_series_thumbnail(user, series).await?);
        }
        self.mosaic(thumbnails).await
    }

    /// Kotlin `MosaicGenerator.createMosaic` of the member thumbnails, repeated in order when
    /// fewer than four.
    async fn mosaic(&self, thumbnails: Vec<Option<Vec<u8>>>) -> Result<Vec<u8>, ArtworkError> {
        let height = ServerSettings::load(&self.db)
            .await?
            .thumbnail_size
            .max_edge();
        blocking::run(move || {
            let images: Vec<&[u8]> = thumbnails
                .iter()
                .cycle()
                .take(MOSAIC_IMAGES)
                .flatten()
                .map(Vec::as_slice)
                .collect();
            komga_image::mosaic(&images, height, ImageType::Jpeg)
        })
        .await?
        .map_err(ArtworkError::Image)
    }

    /// `GET {id}/thumbnails` of a read list or collection.
    pub async fn set_thumbnails<S: PosterSet>(
        &self,
        user: &KomgaUser,
        set: &S,
    ) -> Result<Vec<ThumbnailDetails>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        check_visible(&mut tx, user, set).await?;
        let thumbnails = ThumbnailSetDao::find_all_details(&mut tx, set).await?;
        tx.rollback().await?;
        Ok(thumbnails)
    }

    /// `GET {id}/thumbnails/{thumbnailId}` of a read list or collection.
    pub async fn set_thumbnail_by_id<S: PosterSet>(
        &self,
        user: &KomgaUser,
        set: &S,
        id: &ThumbnailId,
    ) -> Result<Vec<u8>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        check_owned(&mut tx, user, set, id).await?;
        let bytes = ThumbnailSetDao::find_thumbnail::<S>(&mut tx, id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        tx.rollback().await?;
        Ok(bytes)
    }

    /// `POST {id}/thumbnails` of a read list or collection: `addThumbnail` of an uploaded image.
    pub async fn add_set_thumbnail<S: PosterSet>(
        &self,
        user: &KomgaUser,
        set: &S,
        bytes: &[u8],
        selected: bool,
    ) -> Result<ThumbnailDetails, ArtworkError> {
        let mut read_tx = self.db.begin().await?;
        check_visible(&mut read_tx, user, set).await?;
        read_tx.rollback().await?;
        let details = upload::details(bytes, selected).await?;
        let mut tx = self.db.begin_immediate().await?;
        check_visible(&mut tx, user, set).await?;
        ThumbnailSetDao::insert(&mut tx, set, &details, bytes).await?;
        if selected {
            ThumbnailSetDao::mark_selected(&mut tx, set, &details.id).await?;
        }
        tx.record(DomainEvent::ThumbnailSetAdded(set_thumbnail(set, selected)));
        self.events.commit(tx).await?;
        info!(set = %set, thumbnail = %details.id, "Added uploaded thumbnail");
        Ok(details)
    }

    /// `PUT {id}/thumbnails/{thumbnailId}/selected` of a read list or collection.
    pub async fn select_set_thumbnail<S: PosterSet>(
        &self,
        user: &KomgaUser,
        set: &S,
        id: &ThumbnailId,
    ) -> Result<(), ArtworkError> {
        let mut tx = self.db.begin_immediate().await?;
        check_owned(&mut tx, user, set, id).await?;
        ThumbnailSetDao::mark_selected(&mut tx, set, id).await?;
        tx.record(DomainEvent::ThumbnailSetAdded(set_thumbnail(set, true)));
        self.events.commit(tx).await?;
        Ok(())
    }

    /// `DELETE {id}/thumbnails/{thumbnailId}` of a read list or collection: another poster is
    /// selected when the deleted one was.
    pub async fn delete_set_thumbnail<S: PosterSet>(
        &self,
        user: &KomgaUser,
        set: &S,
        id: &ThumbnailId,
    ) -> Result<(), ArtworkError> {
        let mut tx = self.db.begin_immediate().await?;
        check_owned(&mut tx, user, set, id).await?;
        let selected = ThumbnailSetDao::delete::<S>(&mut tx, id).await?;
        house_keeping::set(&mut tx, set).await?;
        tx.record(DomainEvent::ThumbnailSetDeleted(set_thumbnail(
            set, selected,
        )));
        self.events.commit(tx).await?;
        info!(set = %set, thumbnail = %id, "Deleted uploaded thumbnail");
        Ok(())
    }
}

fn set_thumbnail(set: &impl PosterSet, selected: bool) -> ThumbnailRef<SetId> {
    ThumbnailRef {
        owner: set.set_id(),
        selected,
    }
}

async fn check_visible<S: PosterSet>(
    tx: &mut DbTransaction,
    user: &KomgaUser,
    set: &S,
) -> Result<(), ArtworkError> {
    let scope = ContentScope::for_user(user);
    if set.is_visible(tx, Some(&scope)).await? {
        Ok(())
    } else {
        Err(ArtworkError::NotFound)
    }
}

/// Checks that `user` sees the set and that the thumbnail is that set's.
async fn check_owned<S: PosterSet>(
    tx: &mut DbTransaction,
    user: &KomgaUser,
    set: &S,
    id: &ThumbnailId,
) -> Result<(), ArtworkError> {
    check_visible(tx, user, set).await?;
    let owner: S = ThumbnailSetDao::find_owner(tx, id)
        .await?
        .ok_or(ArtworkError::NotFound)?;
    if owner != *set {
        return Err(ArtworkError::OtherOwner);
    }
    Ok(())
}
