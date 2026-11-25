//! In-process publication of committed domain changes.

use komga_db::{Database, DbError, DbTransaction, PosterSet, ReadListDao, SeriesCollectionDao};
use komga_types::{
    CollectionRef, ContentScope, DomainEvent, KomgaUser, LibraryId, ReadListRef, SeriesId, SetId,
};
use tokio::sync::broadcast;

use crate::support::access_policy::{self, Access};

/// Events a subscriber may fall behind by before it misses some.
const CAPACITY: usize = 1024;

/// A bounded broadcast of [`DomainEvent`]s. It is not a durable queue: an event published
/// without subscribers, or beyond what a slow subscriber holds, is lost to it.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<DomainEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self {
            sender: broadcast::Sender::new(CAPACITY),
        }
    }
}

impl EventBus {
    /// Publishes a change; call it only once the change is committed.
    pub fn publish(&self, event: DomainEvent) {
        // Without subscribers nobody is listening, which is not an error.
        let _ = self.sender.send(event);
    }

    /// Commits `tx`, then publishes the events it recorded.
    pub async fn commit(&self, tx: DbTransaction) -> Result<(), DbError> {
        for event in tx.commit_with_events().await? {
            self.publish(event);
        }
        Ok(())
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEvent> {
        self.sender.subscribe()
    }
}

/// Subscriptions to the [`EventBus`] for users, who may see only some events.
#[derive(Clone)]
pub struct EventFeed {
    db: Database,
    events: EventBus,
}

impl EventFeed {
    #[must_use]
    pub fn new(db: Database, events: EventBus) -> Self {
        Self { db, events }
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEvent> {
        self.events.subscribe()
    }

    /// `event` as `user` sees it, if at all. A library must be shared with them and, for a
    /// series or book that still exists, its content allowed; a deleted series has no metadata
    /// left to check, so its deletion and its books' go to everyone sharing the library. A read
    /// list or collection lists only the members they see; one with none is seen only deleted.
    /// Progress is seen only by its user, and a thumbnail by those who see its owner.
    pub async fn visible(
        &self,
        user: &KomgaUser,
        event: &DomainEvent,
    ) -> Result<Option<DomainEvent>, DbError> {
        let has_books = |list: &ReadListRef| !list.book_ids.is_empty();
        let has_series = |collection: &CollectionRef| !collection.series_ids.is_empty();
        Ok(match event {
            DomainEvent::LibraryAdded(library_id)
            | DomainEvent::LibraryUpdated(library_id)
            | DomainEvent::LibraryDeleted(library_id) => {
                user.can_access_library(library_id).then(|| event.clone())
            }
            DomainEvent::SeriesAdded(series) | DomainEvent::SeriesUpdated(series) => self
                .can_see(user, &series.library_id, &series.series_id)
                .await?
                .then(|| event.clone()),
            DomainEvent::BookAdded(book) | DomainEvent::BookUpdated(book) => self
                .can_see(user, &book.library_id, &book.series_id)
                .await?
                .then(|| event.clone()),
            DomainEvent::SeriesDeleted(series) => user
                .can_access_library(&series.library_id)
                .then(|| event.clone()),
            DomainEvent::BookDeleted(book) => user
                .can_access_library(&book.library_id)
                .then(|| event.clone()),
            DomainEvent::ReadProgressChanged(progress)
            | DomainEvent::ReadProgressDeleted(progress) => {
                (user.id == progress.user_id).then(|| event.clone())
            }
            DomainEvent::ReadProgressSeriesChanged(progress)
            | DomainEvent::ReadProgressSeriesDeleted(progress) => {
                (user.id == progress.user_id).then(|| event.clone())
            }
            DomainEvent::SessionsExpired(user_id) => (user.id == *user_id).then(|| event.clone()),
            DomainEvent::BookImported(_) => user.is_admin().then(|| event.clone()),
            DomainEvent::ReadListAdded(list) => Some(self.read_list_seen(user, list).await?)
                .filter(has_books)
                .map(DomainEvent::ReadListAdded),
            DomainEvent::ReadListUpdated(list) => Some(self.read_list_seen(user, list).await?)
                .filter(has_books)
                .map(DomainEvent::ReadListUpdated),
            DomainEvent::ReadListDeleted(list) => Some(DomainEvent::ReadListDeleted(
                self.read_list_seen(user, list).await?,
            )),
            DomainEvent::CollectionAdded(collection) => {
                Some(self.collection_seen(user, collection).await?)
                    .filter(has_series)
                    .map(DomainEvent::CollectionAdded)
            }
            DomainEvent::CollectionUpdated(collection) => {
                Some(self.collection_seen(user, collection).await?)
                    .filter(has_series)
                    .map(DomainEvent::CollectionUpdated)
            }
            DomainEvent::CollectionDeleted(collection) => Some(DomainEvent::CollectionDeleted(
                self.collection_seen(user, collection).await?,
            )),
            DomainEvent::ThumbnailBookAdded(thumbnail)
            | DomainEvent::ThumbnailBookDeleted(thumbnail) => {
                let book = &thumbnail.owner;
                self.can_see(user, &book.library_id, &book.series_id)
                    .await?
                    .then(|| event.clone())
            }
            DomainEvent::ThumbnailSeriesAdded(thumbnail)
            | DomainEvent::ThumbnailSeriesDeleted(thumbnail) => {
                let series = &thumbnail.owner;
                self.can_see(user, &series.library_id, &series.series_id)
                    .await?
                    .then(|| event.clone())
            }
            DomainEvent::ThumbnailSetAdded(thumbnail)
            | DomainEvent::ThumbnailSetDeleted(thumbnail) => self
                .set_seen(user, &thumbnail.owner)
                .await?
                .then(|| event.clone()),
        })
    }

    /// `list` with only the books `user` sees.
    async fn read_list_seen(
        &self,
        user: &KomgaUser,
        list: &ReadListRef,
    ) -> Result<ReadListRef, DbError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        let book_ids = ReadListDao::visible_books(&mut tx, &list.book_ids, &scope).await?;
        tx.rollback().await?;
        Ok(ReadListRef {
            read_list_id: list.read_list_id.clone(),
            book_ids,
        })
    }

    /// `collection` with only the series `user` sees.
    async fn collection_seen(
        &self,
        user: &KomgaUser,
        collection: &CollectionRef,
    ) -> Result<CollectionRef, DbError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        let series_ids =
            SeriesCollectionDao::visible_series(&mut tx, &collection.series_ids, &scope).await?;
        tx.rollback().await?;
        Ok(CollectionRef {
            collection_id: collection.collection_id.clone(),
            series_ids,
        })
    }

    /// Whether `user` sees the read list or collection, which needs a member they see.
    async fn set_seen(&self, user: &KomgaUser, set: &SetId) -> Result<bool, DbError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        let seen = match set {
            SetId::ReadList(id) => id.is_visible(&mut tx, Some(&scope)).await?,
            SetId::Collection(id) => id.is_visible(&mut tx, Some(&scope)).await?,
        };
        tx.rollback().await?;
        Ok(seen)
    }

    /// Only a restricted user looks the series up; one deleted since is not seen.
    async fn can_see(
        &self,
        user: &KomgaUser,
        library_id: &LibraryId,
        series_id: &SeriesId,
    ) -> Result<bool, DbError> {
        if !user.can_access_library(library_id) {
            return Ok(false);
        }
        if !user.restrictions.is_restricted() {
            return Ok(true);
        }
        let mut tx = self.db.begin().await?;
        let access = access_policy::check_series_id(&mut tx, user, series_id).await?;
        tx.rollback().await?;
        Ok(access == Access::Allowed)
    }
}
