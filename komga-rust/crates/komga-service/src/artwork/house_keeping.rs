//! Kotlin `BookLifecycle.thumbnailsHouseKeeping`, `SeriesLifecycle.thumbnailsHouseKeeping` and
//! `ReadListLifecycle` and `SeriesCollectionLifecycle` `thumbnailsHouseKeeping`: drop rows without content, then leave exactly one
//! selected when any remain.
//!
//! Whether a local artwork file still exists is decided by scans ([`forget_local_artwork`]), not
//! here, so that serving a thumbnail never touches the library's storage.

use std::collections::HashSet;

use komga_db::{
    DbError, DbTransaction, PosterSet, ThumbnailBookDao, ThumbnailEntry, ThumbnailSeriesDao,
    ThumbnailSetDao,
};
use komga_types::{BookId, FileUrl, SeriesId, ThumbnailId};
use tracing::{info, warn};

pub(crate) async fn book(tx: &mut DbTransaction, book_id: &BookId) -> Result<(), DbError> {
    let entries = ThumbnailBookDao::find_all_by_book(tx, book_id).await?;
    let mut stored = Vec::new();
    for entry in entries {
        if entry.stored {
            stored.push(entry);
        } else {
            warn!(book = %book_id, thumbnail = %entry.id, "Thumbnail doesn't exist, removing entry");
            ThumbnailBookDao::delete(tx, &entry.id).await?;
        }
    }
    if let Some(keep) = to_select(&stored) {
        ThumbnailBookDao::mark_selected(tx, book_id, keep).await?;
    }
    Ok(())
}

pub(crate) async fn series(tx: &mut DbTransaction, series_id: &SeriesId) -> Result<(), DbError> {
    let entries = ThumbnailSeriesDao::find_all_by_series(tx, series_id).await?;
    let mut stored = Vec::new();
    for entry in entries {
        if entry.stored {
            stored.push(entry);
        } else {
            warn!(series = %series_id, thumbnail = %entry.id, "Thumbnail doesn't exist, removing entry");
            ThumbnailSeriesDao::delete(tx, &entry.id).await?;
        }
    }
    if let Some(keep) = to_select(&stored) {
        ThumbnailSeriesDao::mark_selected(tx, series_id, keep).await?;
    }
    Ok(())
}

/// Read list and collection rows always hold their bytes.
pub(crate) async fn set<S: PosterSet>(tx: &mut DbTransaction, set: &S) -> Result<(), DbError> {
    let entries = ThumbnailSetDao::find_all(tx, set).await?;
    if let Some(keep) = to_select(&entries) {
        ThumbnailSetDao::mark_selected(tx, set, keep).await?;
    }
    Ok(())
}

/// The row to mark selected: the first selected when several are, the first row when none is.
fn to_select(stored: &[ThumbnailEntry]) -> Option<&ThumbnailId> {
    let selected: Vec<&ThumbnailEntry> = stored.iter().filter(|entry| entry.selected).collect();
    match selected.as_slice() {
        [] => stored.first().map(|entry| &entry.id),
        [_] => None,
        [first, ..] => Some(&first.id),
    }
}

/// Drops the thumbnails read from sidecars a scan found removed, and selects another thumbnail
/// for their books and series.
pub(crate) async fn forget_local_artwork(
    tx: &mut DbTransaction,
    urls: &[&FileUrl],
) -> Result<(), DbError> {
    let book_ids: HashSet<BookId> = ThumbnailBookDao::delete_sidecars_by_urls(tx, urls)
        .await?
        .into_iter()
        .collect();
    for book_id in &book_ids {
        info!(book = %book_id, "Local artwork removed from disk, selecting another thumbnail");
        book(tx, book_id).await?;
    }
    let series_ids: HashSet<SeriesId> = ThumbnailSeriesDao::delete_sidecars_by_urls(tx, urls)
        .await?
        .into_iter()
        .collect();
    for series_id in &series_ids {
        info!(series = %series_id, "Local artwork removed from disk, selecting another thumbnail");
        series(tx, series_id).await?;
    }
    Ok(())
}
