//! Kotlin `KoboController.syncLibrary`: the changes since the device's last sync, page by page.

use komga_db::{KoboDao, SyncPointDao};
use komga_types::{
    ApiKeyId, BookId, KoboBook, KoboReadProgress, KomgaUser, SyncAnchors, SyncPointBook,
    SyncPointBooks, SyncPointId, SyncPointReadList, SyncPointReadLists,
};

use super::{KoboError, KoboSyncService};
use crate::SyncPointService;
use crate::sync::points::SyncRound;

/// One page of a library sync. A full sync hands everything out as added.
#[derive(Debug, Default)]
pub struct KoboChanges {
    pub books_added: Vec<SyncPointBook>,
    pub books_changed: Vec<SyncPointBook>,
    pub books_removed: Vec<SyncPointBook>,
    pub books_progress_changed: Vec<SyncPointBook>,
    pub read_lists_added: Vec<SyncPointReadList>,
    pub read_lists_changed: Vec<SyncPointReadList>,
    pub read_lists_removed: Vec<SyncPointReadList>,
    /// Of the books added and changed; a book deleted since the snapshot has none.
    pub metadata: Vec<KoboBook>,
    /// The user's progress on the books added, changed and whose progress changed.
    pub progress: Vec<KoboReadProgress>,
    /// `(read list ID, book ID)` of the read lists added and changed, as snapshotted.
    pub read_list_books: Vec<(String, BookId)>,
}

/// A page handed out but not yet acknowledged by new anchors.
#[derive(Debug)]
pub struct KoboSync {
    pub round: SyncRound,
    pub changes: KoboChanges,
    /// Items remain for another page.
    pub more: bool,
}

/// The items a page may still take. Unlike Kotlin, a step runs even with nothing left to take,
/// so a page never ends the sync while a later step still has items.
struct Budget<'a> {
    sync_points: &'a SyncPointService,
    to: &'a SyncPointId,
    remaining: u32,
    more: bool,
}

impl Budget<'_> {
    async fn books(&mut self, which: SyncPointBooks<'_>) -> Result<Vec<SyncPointBook>, KoboError> {
        let (books, total) = self
            .sync_points
            .take_books(self.to, which, self.remaining)
            .await?;
        self.spend(books.len(), total);
        Ok(books)
    }

    async fn read_lists(
        &mut self,
        which: SyncPointReadLists<'_>,
    ) -> Result<Vec<SyncPointReadList>, KoboError> {
        let (read_lists, total) = self
            .sync_points
            .take_read_lists(self.to, which, self.remaining)
            .await?;
        self.spend(read_lists.len(), total);
        Ok(read_lists)
    }

    fn spend(&mut self, taken: usize, total: u64) {
        let taken = u32::try_from(taken).expect("A page holds at most the item limit");
        self.remaining -= taken;
        self.more |= total > u64::from(taken);
    }
}

impl KoboSyncService {
    /// `GET /kobo/{key}/v1/library/sync`: resumes the ongoing sync point or takes a new one,
    /// and hands out the next page of changes since the last sync handed out in full.
    pub async fn sync(
        &self,
        user: &KomgaUser,
        api_key_id: Option<&ApiKeyId>,
        anchors: &SyncAnchors,
    ) -> Result<KoboSync, KoboError> {
        let round = self.sync_points.start(user, api_key_id, anchors).await?;
        let mut budget = Budget {
            sync_points: &self.sync_points,
            to: &round.to.id,
            remaining: self.sync_item_limit,
            more: false,
        };
        let mut changes = KoboChanges::default();
        if let Some(from) = &round.from {
            let from = &from.id;
            changes.books_added = budget.books(SyncPointBooks::AddedSince(from)).await?;
            changes.books_changed = budget.books(SyncPointBooks::ChangedSince(from)).await?;
            changes.books_removed = budget.books(SyncPointBooks::RemovedSince(from)).await?;
            changes.books_progress_changed = budget
                .books(SyncPointBooks::ReadProgressChangedSince(from))
                .await?;
            let added = SyncPointReadLists::AddedSince(from);
            changes.read_lists_added = budget.read_lists(added).await?;
            let changed = SyncPointReadLists::ChangedSince(from);
            changes.read_lists_changed = budget.read_lists(changed).await?;
            let removed = SyncPointReadLists::RemovedSince(from);
            changes.read_lists_removed = budget.read_lists(removed).await?;
        } else {
            changes.books_added = budget.books(SyncPointBooks::All).await?;
            changes.read_lists_added = budget.read_lists(SyncPointReadLists::All).await?;
        }
        let more = budget.more;
        self.look_up(user, &round.to.id, &mut changes).await?;
        Ok(KoboSync {
            round,
            changes,
            more,
        })
    }

    /// The anchors the device sends back next: the page's sync point stays ongoing while more
    /// remains, and otherwise replaces the baseline.
    pub async fn finish_sync(
        &self,
        round: SyncRound,
        more: bool,
    ) -> Result<SyncAnchors, KoboError> {
        Ok(self.sync_points.finish(round, more).await?)
    }

    async fn look_up(
        &self,
        user: &KomgaUser,
        to: &SyncPointId,
        changes: &mut KoboChanges,
    ) -> Result<(), KoboError> {
        let shown: Vec<BookId> = changes
            .books_added
            .iter()
            .chain(&changes.books_changed)
            .map(|book| book.book_id.clone())
            .collect();
        let mut with_progress = shown.clone();
        with_progress.extend(
            changes
                .books_progress_changed
                .iter()
                .map(|book| book.book_id.clone()),
        );
        let read_lists: Vec<&str> = changes
            .read_lists_added
            .iter()
            .chain(&changes.read_lists_changed)
            .map(|read_list| read_list.read_list_id.as_str())
            .collect();
        let mut tx = self.db.begin().await?;
        changes.metadata = KoboDao::find_books(&mut tx, &shown).await?;
        changes.progress = KoboDao::find_read_progress(&mut tx, &user.id, &with_progress).await?;
        changes.read_list_books =
            SyncPointDao::find_read_list_books(&mut tx, to, &read_lists).await?;
        tx.rollback().await?;
        Ok(())
    }
}
