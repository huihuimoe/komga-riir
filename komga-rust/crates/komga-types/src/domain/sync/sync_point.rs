//! Kotlin `SyncPoint`: a snapshot of the books and read lists a user syncs, and which of them
//! were handed out.

use jiff::civil::DateTime;

use crate::{ApiKeyId, BookId, SyncPointId, ThumbnailId, UserId};

/// The read list ID of the user's On Deck books.
pub const ON_DECK_READ_LIST_ID: &str = "KOMGA-ONDECK";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPoint {
    pub id: SyncPointId,
    pub user_id: UserId,
    pub api_key_id: Option<ApiKeyId>,
    /// UTC.
    pub created_date: DateTime,
}

/// Where a client's sync stands, as it hands back between requests (Kotlin `KomgaSyncToken`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncAnchors {
    /// The sync point still being handed out page by page.
    pub ongoing: Option<SyncPointId>,
    /// The last sync point handed out in full: what the next sync point is compared with.
    pub last_synced: Option<SyncPointId>,
}

/// A book as it was when the sync point was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPointBook {
    pub book_id: BookId,
    pub created_date: DateTime,
    pub last_modified_date: DateTime,
    pub file_last_modified: DateTime,
    pub file_size: i64,
    /// Empty until the file is hashed.
    pub file_hash: String,
    pub metadata_last_modified_date: DateTime,
    pub thumbnail_id: Option<ThumbnailId>,
    pub synced: bool,
}

/// A read list as it was when the sync point was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPointReadList {
    /// A read list ID, or [`ON_DECK_READ_LIST_ID`].
    pub read_list_id: String,
    pub name: String,
    pub created_date: DateTime,
    pub last_modified_date: DateTime,
    pub synced: bool,
}

/// Which books of a sync point to take, compared with an earlier one when given.
#[derive(Debug, Clone, Copy)]
pub enum SyncPointBooks<'a> {
    All,
    /// Books absent from the earlier sync point.
    AddedSince(&'a SyncPointId),
    /// Books whose file, metadata or selected thumbnail changed.
    ChangedSince(&'a SyncPointId),
    /// Books of the earlier sync point that are absent now.
    RemovedSince(&'a SyncPointId),
    /// Unchanged books whose read progress changed.
    ReadProgressChangedSince(&'a SyncPointId),
}

/// Which read lists of a sync point to take, compared with an earlier one when given.
#[derive(Debug, Clone, Copy)]
pub enum SyncPointReadLists<'a> {
    All,
    AddedSince(&'a SyncPointId),
    /// Read lists renamed or modified.
    ChangedSince(&'a SyncPointId),
    RemovedSince(&'a SyncPointId),
}
