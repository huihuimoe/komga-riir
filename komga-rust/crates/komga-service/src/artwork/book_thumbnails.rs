//! Kotlin `BookController` poster routes by thumbnail ID and upload, and
//! `BookLifecycle.{getThumbnailBytesByThumbnailId,deleteThumbnailForBook}`.

use komga_db::{BookDao, DbTransaction, ThumbnailBookDao, ThumbnailContent};
use komga_types::{
    BookId, BookRef, DomainEvent, KomgaUser, ThumbnailDetails, ThumbnailId, ThumbnailRef,
    ThumbnailType,
};
use tracing::info;

use super::{ArtworkError, ArtworkService, ThumbnailOwner, check, house_keeping, upload};
use crate::support::access_policy;

impl ArtworkService {
    /// `GET /api/v1/books/{bookId}/thumbnails`.
    pub async fn book_thumbnails(
        &self,
        user: &KomgaUser,
        book_id: &BookId,
    ) -> Result<Vec<ThumbnailDetails>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        check(access_policy::check_book_id(&mut tx, user, book_id).await?)?;
        let thumbnails = ThumbnailBookDao::find_all_details_by_book(&mut tx, book_id).await?;
        tx.rollback().await?;
        Ok(thumbnails)
    }

    /// `GET /api/v1/books/{bookId}/thumbnails/{thumbnailId}`. As in Kotlin, the book and the
    /// thumbnail's own book are checked separately, and need not be the same.
    pub async fn book_thumbnail_by_id(
        &self,
        user: &KomgaUser,
        book_id: &BookId,
        id: &ThumbnailId,
    ) -> Result<Vec<u8>, ArtworkError> {
        let mut tx = self.db.begin().await?;
        check(access_policy::check_book_id(&mut tx, user, book_id).await?)?;
        let (owner, _) = ThumbnailBookDao::find_owner(&mut tx, id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        check(access_policy::check_book_id(&mut tx, user, &owner).await?)?;
        let content = ThumbnailBookDao::find_content(&mut tx, id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        tx.rollback().await?;
        self.thumbnail_bytes(&owner, content).await
    }

    /// The Kobo cover by thumbnail ID alone, with its media type.
    pub async fn book_thumbnail_by_id_only(
        &self,
        user: &KomgaUser,
        id: &ThumbnailId,
    ) -> Result<(Vec<u8>, String), ArtworkError> {
        let mut tx = self.db.begin().await?;
        let (owner, _) = ThumbnailBookDao::find_owner(&mut tx, id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        check(access_policy::check_book_id(&mut tx, user, &owner).await?)?;
        let content = ThumbnailBookDao::find_content(&mut tx, id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        tx.rollback().await?;
        let media_type = content.media_type.clone();
        Ok((self.thumbnail_bytes(&owner, content).await?, media_type))
    }

    async fn thumbnail_bytes(
        &self,
        owner: &BookId,
        content: ThumbnailContent,
    ) -> Result<Vec<u8>, ArtworkError> {
        match (content.thumbnail, content.url) {
            (Some(bytes), _) => Ok(bytes),
            (None, Some(url)) => self
                .cache_kotlin_sidecar(ThumbnailOwner::Book(owner), &content.id, &url)
                .await?
                .ok_or(ArtworkError::NotFound),
            (None, None) => Err(ArtworkError::NotFound),
        }
    }

    /// `POST /api/v1/books/{bookId}/thumbnails`: `addThumbnailForBook` of an uploaded image with
    /// `YES` or `NO`. As in Kotlin, the returned `selected` is the requested one, even when the
    /// house keeping then selects the upload as the book's only thumbnail.
    pub async fn add_book_thumbnail(
        &self,
        book_id: &BookId,
        bytes: &[u8],
        selected: bool,
    ) -> Result<ThumbnailDetails, ArtworkError> {
        // Check existence before probing so missing books fail with 404 rather than 415,
        // without holding a SQLite write lock across image/media-type probing.
        let mut read_tx = self.db.begin().await?;
        let exists = BookDao::find_by_id(&mut read_tx, book_id).await?.is_some();
        read_tx.rollback().await?;
        if !exists {
            return Err(ArtworkError::NotFound);
        }
        let details = upload::details(bytes, selected).await?;
        let mut tx = self.db.begin_immediate().await?;
        let book = BookDao::find_by_id(&mut tx, book_id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        ThumbnailBookDao::insert(&mut tx, book_id, &upload::row(&details, bytes)).await?;
        if selected {
            ThumbnailBookDao::mark_selected(&mut tx, book_id, &details.id).await?;
        } else {
            house_keeping::book(&mut tx, book_id).await?;
        }
        tx.record(DomainEvent::ThumbnailBookAdded(ThumbnailRef {
            owner: BookRef::from(&book),
            selected,
        }));
        self.events.commit(tx).await?;
        info!(book = %book_id, thumbnail = %details.id, "Added uploaded thumbnail");
        Ok(details)
    }

    /// `PUT /api/v1/books/{bookId}/thumbnails/{thumbnailId}/selected`.
    pub async fn select_book_thumbnail(
        &self,
        book_id: &BookId,
        id: &ThumbnailId,
    ) -> Result<(), ArtworkError> {
        let mut tx = self.db.begin_immediate().await?;
        let (owner, _) = owned_kind(&mut tx, book_id, id).await?;
        ThumbnailBookDao::mark_selected(&mut tx, book_id, id).await?;
        tx.record(DomainEvent::ThumbnailBookAdded(ThumbnailRef {
            owner,
            selected: true,
        }));
        self.events.commit(tx).await?;
        Ok(())
    }

    /// `DELETE /api/v1/books/{bookId}/thumbnails/{thumbnailId}`: only uploaded thumbnails, after
    /// which another one is selected if needed.
    pub async fn delete_book_thumbnail(
        &self,
        book_id: &BookId,
        id: &ThumbnailId,
    ) -> Result<(), ArtworkError> {
        let mut tx = self.db.begin_immediate().await?;
        let (owner, kind) = owned_kind(&mut tx, book_id, id).await?;
        if kind != ThumbnailType::UserUploaded {
            return Err(ArtworkError::NotUploaded);
        }
        let selected = ThumbnailBookDao::delete(&mut tx, id).await?;
        house_keeping::book(&mut tx, book_id).await?;
        tx.record(DomainEvent::ThumbnailBookDeleted(ThumbnailRef {
            owner,
            selected,
        }));
        self.events.commit(tx).await?;
        info!(book = %book_id, thumbnail = %id, "Deleted uploaded thumbnail");
        Ok(())
    }
}

/// An existing book and the type of its thumbnail, which must be that book's.
async fn owned_kind(
    tx: &mut DbTransaction,
    book_id: &BookId,
    id: &ThumbnailId,
) -> Result<(BookRef, ThumbnailType), ArtworkError> {
    let book = BookDao::find_by_id(tx, book_id)
        .await?
        .ok_or(ArtworkError::NotFound)?;
    let (owner, kind) = ThumbnailBookDao::find_owner(tx, id)
        .await?
        .ok_or(ArtworkError::NotFound)?;
    if owner != *book_id {
        return Err(ArtworkError::OtherOwner);
    }
    Ok((BookRef::from(&book), kind))
}
