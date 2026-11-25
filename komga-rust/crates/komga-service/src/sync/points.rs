//! Kotlin `SyncPointLifecycle`: snapshots of the EPUB books a user syncs, handed out page by page.

use komga_db::{Database, DbError, SyncPointDao};
use komga_types::{
    ApiKeyId, BookCondition, BooleanOperator, ContentScope, Equality, KomgaUser, MediaProfile,
    MediaStatus, SyncAnchors, SyncPoint, SyncPointBook, SyncPointBooks, SyncPointId,
    SyncPointReadList, SyncPointReadLists, UserId,
};

/// The sync points one sync request works between.
#[derive(Debug)]
pub struct SyncRound {
    /// Handed out from: the ongoing sync point, or a new one.
    pub to: SyncPoint,
    /// Compared with: the last sync point handed out in full. `None` means a full sync.
    pub from: Option<SyncPoint>,
}

#[derive(Clone)]
pub struct SyncPointService {
    db: Database,
}

impl SyncPointService {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// `createSyncPoint`: the ready EPUB books `user` may see, with the user's On Deck books
    /// as a read list. Kotlin's only caller syncs all libraries, so there is no library filter.
    pub async fn create(
        &self,
        user: &KomgaUser,
        api_key_id: Option<&ApiKeyId>,
    ) -> Result<SyncPoint, DbError> {
        let scope = ContentScope::for_user(user);
        let condition = BookCondition::AllOf(vec![
            BookCondition::MediaStatus(Equality::Is {
                value: MediaStatus::Ready,
            }),
            BookCondition::MediaProfile(Equality::Is {
                value: MediaProfile::Epub,
            }),
            BookCondition::Deleted(BooleanOperator::IsFalse),
        ]);
        let mut tx = self.db.begin_immediate().await?;
        let sync_point =
            SyncPointDao::create(&mut tx, &user.id, api_key_id, &scope, &condition).await?;
        SyncPointDao::add_on_deck(&mut tx, &sync_point, &scope).await?;
        tx.commit().await?;
        Ok(sync_point)
    }

    /// `takeBooks*`: up to `size` books not handed out yet, and how many there were.
    pub async fn take_books(
        &self,
        to: &SyncPointId,
        books: SyncPointBooks<'_>,
        size: u32,
    ) -> Result<(Vec<SyncPointBook>, u64), DbError> {
        let mut tx = self.db.begin_immediate().await?;
        let taken = SyncPointDao::take_books(&mut tx, to, books, size).await?;
        tx.commit().await?;
        Ok(taken)
    }

    /// `takeReadLists*`: as [`Self::take_books`], for read lists.
    pub async fn take_read_lists(
        &self,
        to: &SyncPointId,
        read_lists: SyncPointReadLists<'_>,
        size: u32,
    ) -> Result<(Vec<SyncPointReadList>, u64), DbError> {
        let mut tx = self.db.begin_immediate().await?;
        let taken = SyncPointDao::take_read_lists(&mut tx, to, read_lists, size).await?;
        tx.commit().await?;
        Ok(taken)
    }

    /// Resolves `anchors` for `user`. An anchor that is gone or belongs to another user counts
    /// as absent, so a client never reads someone else's sync and a lost anchor means a fresh
    /// sync point (ongoing) or a full sync (last synced).
    pub async fn start(
        &self,
        user: &KomgaUser,
        api_key_id: Option<&ApiKeyId>,
        anchors: &SyncAnchors,
    ) -> Result<SyncRound, DbError> {
        let to = match self.owned(user, anchors.ongoing.as_ref()).await? {
            Some(ongoing) => ongoing,
            None => self.create(user, api_key_id).await?,
        };
        let from = self.owned(user, anchors.last_synced.as_ref()).await?;
        Ok(SyncRound { to, from })
    }

    /// The anchors to hand back after `round`. While `more` remains, `to` stays ongoing and a
    /// retried request continues it; once done, `to` becomes the last synced point and the
    /// previous one is deleted.
    pub async fn finish(&self, round: SyncRound, more: bool) -> Result<SyncAnchors, DbError> {
        if more {
            return Ok(SyncAnchors {
                ongoing: Some(round.to.id),
                last_synced: round.from.map(|from| from.id),
            });
        }
        if let Some(from) = round.from {
            let mut tx = self.db.begin_immediate().await?;
            SyncPointDao::delete(&mut tx, &from.id).await?;
            tx.commit().await?;
        }
        Ok(SyncAnchors {
            ongoing: None,
            last_synced: Some(round.to.id),
        })
    }

    /// `deleteByUserId`, or only the sync points of `api_key_ids` when given: the next sync of
    /// those clients starts from scratch.
    pub async fn delete_for_user(
        &self,
        user_id: &UserId,
        api_key_ids: Option<&[ApiKeyId]>,
    ) -> Result<(), DbError> {
        let mut tx = self.db.begin_immediate().await?;
        SyncPointDao::delete_by_user(&mut tx, user_id, api_key_ids).await?;
        tx.commit().await
    }

    async fn owned(
        &self,
        user: &KomgaUser,
        id: Option<&SyncPointId>,
    ) -> Result<Option<SyncPoint>, DbError> {
        let Some(id) = id else { return Ok(None) };
        let sync_point = SyncPointDao::find_by_id(&self.db, id).await?;
        Ok(sync_point.filter(|sync_point| sync_point.user_id == user.id))
    }
}
