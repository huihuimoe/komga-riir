//! The sidecar part of Kotlin `LibraryContentLifecycle.scanRootFolder`.

use std::collections::{HashMap, HashSet};

use jiff::civil::DateTime;
use komga_db::{BookDao, DbError, DbTransaction, SeriesDao, SidecarDao};
use komga_scheduler::Task;
use komga_types::{FileUrl, Library, Sidecar, SidecarKind};
use tracing::info;

use crate::artwork::house_keeping;
use crate::library::scanning::file_system_scanner::Unread;
use crate::support::file_time::same_millis;

/// Stores new and changed sidecars, returning refreshes for those whose series or book exists,
/// and forgets the library's sidecars no longer on disk outside `unread`, with the thumbnails read
/// from them.
pub(super) async fn reconcile(
    tx: &mut DbTransaction,
    library: &Library,
    sidecars: &[Sidecar],
    unread: &Unread,
) -> Result<Vec<Task>, DbError> {
    // Kotlin compares with the sidecars of every library.
    let stored = SidecarDao::find_all(tx).await?;
    let stored_times: HashMap<&FileUrl, DateTime> = stored
        .iter()
        .map(|sidecar| (&sidecar.url, sidecar.last_modified_time))
        .collect();
    let mut tasks = Vec::new();
    for sidecar in sidecars {
        let unchanged = stored_times
            .get(&sidecar.url)
            .is_some_and(|&time| same_millis(time, sidecar.last_modified_time));
        if unchanged {
            continue;
        }
        if let Some(task) = sidecar_refresh(tx, library, sidecar).await? {
            info!(sidecar = %sidecar.url, ?task, "Sidecar changed on disk");
            tasks.push(task);
        }
        SidecarDao::save(tx, &library.id, sidecar).await?;
    }
    let scanned: HashSet<&FileUrl> = sidecars.iter().map(|sidecar| &sidecar.url).collect();
    let gone: Vec<&FileUrl> = stored
        .iter()
        .filter(|sidecar| {
            sidecar.library_id == library.id
                && !scanned.contains(&sidecar.url)
                && !unread.contains(&sidecar.url)
        })
        .map(|sidecar| &sidecar.url)
        .collect();
    SidecarDao::delete_by_library_and_urls(tx, &library.id, &gone).await?;
    house_keeping::forget_local_artwork(tx, &gone).await?;
    Ok(tasks)
}

/// The refresh a changed sidecar triggers for its series or book, if not deleted.
async fn sidecar_refresh(
    tx: &mut DbTransaction,
    library: &Library,
    sidecar: &Sidecar,
) -> Result<Option<Task>, DbError> {
    let (library_id, parent) = (&library.id, &sidecar.parent_url);
    let task = match sidecar.kind {
        SidecarKind::SeriesArtwork => {
            SeriesDao::find_not_deleted_id_by_library_and_url(tx, library_id, parent)
                .await?
                .map(Task::refresh_series_local_artwork)
        }
        SidecarKind::SeriesMetadata => {
            SeriesDao::find_not_deleted_id_by_library_and_url(tx, library_id, parent)
                .await?
                .map(Task::refresh_series_metadata)
        }
        SidecarKind::BookArtwork => {
            BookDao::find_not_deleted_id_by_library_and_url(tx, library_id, parent)
                .await?
                .map(Task::refresh_book_local_artwork)
        }
    };
    Ok(task)
}
