//! Kotlin `LibraryContentLifecycle.scanRootFolder`: reconciles a scan of the library root with
//! the stored series and books in one transaction.

use std::collections::{HashMap, HashSet};
use std::io;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use komga_db::{
    BookDao, DbError, DbTransaction, LibraryDao, MediaDao, SearchIndexPendingDao, SeriesDao,
};
use komga_scheduler::{Task, TaskKind, TaskQueue};
use komga_types::{Book, DomainEvent, FileUrl, Library, LibraryId, SearchEntity, Series, SeriesId};
use tracing::{debug, info, warn};

use super::file_system_scanner::{self, ScanError, ScanResult};
use super::library_content_service::{LibraryContentError, LibraryContentService};
use crate::books::hashing;
use crate::library::series;
use crate::support::file_time::same_millis;
use restore::Hashes;
use tokio_util::sync::CancellationToken;

mod missing;
mod restore;
mod sidecars;

impl LibraryContentService {
    /// Scans the library root and stores new series and books. Returns the scanned library, or
    /// `None` when it was deleted since the task was queued. An inaccessible root marks the
    /// library unavailable and fails.
    pub async fn scan_root_folder(
        &self,
        id: &LibraryId,
        scan_deep: bool,
        stop: &CancellationToken,
    ) -> Result<Option<Library>, LibraryContentError> {
        let mut tx = self.db.begin().await?;
        let library = LibraryDao::find_by_id(&mut tx, id).await?;
        tx.rollback().await?;
        let Some(mut library) = library else {
            warn!(library = %id, "Cannot scan: Library does not exist");
            return Ok(None);
        };
        info!(library = %id, root = %library.root.display(), "Scan root folder for library");
        let mut scan = {
            let library = library.clone();
            tokio::task::spawn_blocking(move || file_system_scanner::scan_root_folder(&library))
                .await?
        };
        if scan.as_ref().is_ok_and(|scanned| scanned.series.is_empty())
            && self.has_live_series(id).await?
        {
            scan = Err(ScanError::Emptied(library.root.clone()));
        }
        let unavailable = match &scan {
            Ok(_) => None,
            Err(_) => Some(
                Timestamp::now()
                    .to_zoned_datetime(TimeZone::system())
                    .datetime(),
            ),
        };
        if unavailable.is_some() || library.unavailable_date.is_some() {
            library.unavailable_date = unavailable;
            let mut tx = self.db.begin_immediate().await?;
            LibraryDao::update(&mut tx, &library).await?;
            tx.record(DomainEvent::LibraryUpdated(library.id.clone()));
            self.events.commit(tx).await?;
        }
        let scanned = scan?;

        // Hashing reads whole files, so it runs between attempts rather than inside the write
        // transaction. Each retry knows more hashes, so the loop ends.
        let mut hashes = HashMap::new();
        loop {
            let mut tx = self.db.begin_immediate().await?;
            match reconcile(&mut tx, &library, &scanned, scan_deep, &hashes).await? {
                Reconciled::Done(tasks) => {
                    TaskQueue::stage(&mut tx, &tasks).await?;
                    self.events.commit(tx).await?;
                    self.search.notify();
                    break;
                }
                Reconciled::NeedsHashes(books) => {
                    tx.rollback().await?;
                    for book in books {
                        if stop.is_cancelled() {
                            return Err(LibraryContentError::Interrupted);
                        }
                        let path = book.url.to_path().expect("Scanned books have file URLs");
                        let hash = match hashing::file_hash(&path).await {
                            Ok(hash) => hash,
                            // No retry reads a file gone or forbidden since the walk. Like an
                            // unhashed file, it restores nothing, and a change resets its media.
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                                ) =>
                            {
                                warn!(book = %book.url, %error, "Could not hash book");
                                String::new()
                            }
                            Err(error) => return Err(LibraryContentError::Hash(error)),
                        };
                        hashes.insert(book.url, hash);
                    }
                }
            }
        }
        self.tasks.hand_over(&self.db).await;

        if library.empty_trash_after_scan {
            self.empty_trash(id).await?;
        } else {
            self.cleanup_empty_sets().await?;
        }
        info!(library = %id, "Library updated");
        Ok(Some(library))
    }
}

impl LibraryContentService {
    async fn has_live_series(&self, id: &LibraryId) -> Result<bool, DbError> {
        let mut tx = self.db.begin().await?;
        let series = SeriesDao::find_all_by_library(&mut tx, id).await?;
        tx.rollback().await?;
        Ok(series.iter().any(|series| series.deleted_date.is_none()))
    }
}

enum Reconciled {
    /// The tasks to submit after the commit.
    Done(Vec<Task>),
    /// Changed or new books whose file hash, missing from the given hashes, decides their update
    /// or restoration; the transaction must be rolled back.
    NeedsHashes(Vec<Book>),
}

/// Applies the scan, with `hashes` holding the file hashes of scanned books by URL.
async fn reconcile(
    tx: &mut DbTransaction,
    library: &Library,
    scanned: &ScanResult,
    scan_deep: bool,
    hashes: &HashMap<FileUrl, String>,
) -> Result<Reconciled, LibraryContentError> {
    let ScanResult {
        series: scanned,
        sidecars,
        unread,
    } = scanned;
    let stored_series = SeriesDao::find_all_by_library(tx, &library.id).await?;
    let stored_books = BookDao::find_all_by_library(tx, &library.id).await?;
    let lost_books =
        missing::soft_delete(tx, &stored_series, &stored_books, scanned, unread).await?;
    let mut to_sort: Vec<SeriesId> = lost_books.iter().map(|series| series.id.clone()).collect();
    let lost_books: HashSet<&FileUrl> = lost_books.into_iter().map(|series| &series.url).collect();

    // Series soft deleted above are no longer on disk, so none of them is scanned.
    let live_series: HashMap<&FileUrl, &Series> = stored_series
        .iter()
        .filter(|series| series.deleted_date.is_none())
        .map(|series| (&series.url, series))
        .collect();
    let mut hashes = Hashes {
        known: hashes,
        missing: Vec::new(),
    };
    let mut tasks = Vec::new();
    for (new_series, new_books) in scanned {
        let Some(existing) = live_series.get(&new_series.url) else {
            info!(series = %new_series.url, "Adding new series");
            series::create_series(tx, new_series).await?;
            series::add_books(tx, new_series, new_books).await?;
            let restored = restore::try_restore(
                tx,
                new_series,
                new_books,
                /* new_series= */ true,
                &mut hashes,
            )
            .await?;
            tasks.extend(restored);
            to_sort.push(new_series.id.clone());
            continue;
        };
        let change = SeriesChange {
            existing,
            new_series,
            new_books,
            lost_books: lost_books.contains(&existing.url),
        };
        if update_series(
            tx,
            &change,
            &stored_books,
            scan_deep,
            &mut hashes,
            &mut tasks,
        )
        .await?
        {
            to_sort.push(existing.id.clone());
        }
    }
    if !hashes.missing.is_empty() {
        return Ok(Reconciled::NeedsHashes(hashes.missing));
    }

    let mut sorted = HashSet::new();
    for series_id in to_sort {
        if sorted.insert(series_id.clone()) {
            for refresh in series::sort_books(tx, &series_id).await? {
                push_refresh(&mut tasks, refresh);
            }
            tasks.push(Task::refresh_series_metadata(series_id));
        }
    }
    tasks.extend(sidecars::reconcile(tx, library, sidecars, unread).await?);
    Ok(Reconciled::Done(tasks))
}

/// The queue keeps one refresh per book, so a restored book that the sort renumbers keeps its
/// title refresh by joining both capabilities.
fn push_refresh(tasks: &mut Vec<Task>, refresh: Task) {
    if let Task {
        kind: TaskKind::RefreshBookMetadata {
            book_id,
            capabilities: added,
            ..
        },
        ..
    } = &refresh
        && let Some(Task {
            kind: TaskKind::RefreshBookMetadata {
                capabilities, ..
            },
            ..
        }) = tasks.iter_mut().find(
            |task| matches!(task, Task { kind: TaskKind::RefreshBookMetadata { book_id: id, .. }, .. } if id == book_id),
        )
    {
        for capability in added {
            if !capabilities.contains(capability) {
                capabilities.push(*capability);
            }
        }
        return;
    }
    tasks.push(refresh);
}

/// A stored live series found again by the scan.
struct SeriesChange<'a> {
    existing: &'a Series,
    new_series: &'a Series,
    new_books: &'a [Book],
    /// Lost books reveal a change the directory time may hide, for example behind an NFS/SMB
    /// cache.
    lost_books: bool,
}

/// Stores the new time of a series changed on disk and, when changed or deep, its new books,
/// adding the title refreshes of restored books to `tasks`. Returns whether the series needs
/// sorting. Changed books whose update needs a hash missing from `hashes` are left alone.
async fn update_series(
    tx: &mut DbTransaction,
    change: &SeriesChange<'_>,
    stored_books: &[Book],
    scan_deep: bool,
    hashes: &mut Hashes<'_>,
    tasks: &mut Vec<Task>,
) -> Result<bool, LibraryContentError> {
    let SeriesChange {
        existing,
        new_series,
        new_books,
        lost_books,
    } = *change;
    let changed =
        lost_books || !same_millis(new_series.file_last_modified, existing.file_last_modified);
    if changed {
        info!(series = %existing.url, "Series changed on disk, updating");
        SeriesDao::update_file_state(tx, &existing.id, new_series.file_last_modified, None).await?;
    }
    if !(scan_deep || changed) {
        return Ok(false);
    }
    let existing_books: HashMap<&FileUrl, &Book> = stored_books
        .iter()
        .filter(|book| book.series_id == existing.id && book.deleted_date.is_none())
        .map(|book| (&book.url, book))
        .collect();
    let mut to_add = Vec::new();
    for new_book in new_books {
        match existing_books.get(&new_book.url) {
            Some(existing_book)
                if !same_millis(
                    new_book.file_last_modified,
                    existing_book.file_last_modified,
                ) =>
            {
                let hash = if existing_book.file_size == new_book.file_size
                    && !existing_book.file_hash.is_empty()
                {
                    let Some(hash) = hashes.get(new_book) else {
                        continue;
                    };
                    Some(hash)
                } else {
                    None
                };
                update_changed_book(tx, existing_book, new_book, hash).await?;
            }
            Some(_) => {}
            None => to_add.push(new_book.clone()),
        }
    }
    debug!(series = %existing.url, books = to_add.len(), "Adding new books");
    series::add_books(tx, existing, &to_add).await?;
    let restored =
        restore::try_restore(tx, existing, &to_add, /* new_series= */ false, hashes).await?;
    tasks.extend(restored);
    Ok(true)
}

/// Stores the new file state of a book changed on disk. `hash` is the new file hash, which Kotlin
/// computes only when the size is unchanged and a hash is stored. Unless it equals the stored hash,
/// the media is marked `OUTDATED`, so that its analysis is redone rather than kept for the replaced
/// content.
async fn update_changed_book(
    tx: &mut DbTransaction,
    existing: &Book,
    new_book: &Book,
    hash: Option<&str>,
) -> Result<(), DbError> {
    let mut updated = Book {
        file_last_modified: new_book.file_last_modified,
        file_size: new_book.file_size,
        ..existing.clone()
    };
    if hash == Some(existing.file_hash.as_str()) {
        info!(book = %existing.url, "Book changed on disk, but still has the same hash, no need to reset media status");
        return BookDao::update(tx, &updated).await;
    }
    info!(book = %existing.url, "Book changed on disk, update and reset media status");
    updated.file_hash = hash.unwrap_or_default().to_owned();
    updated.file_hash_koreader = String::new();
    MediaDao::update_status(tx, &existing.id, "OUTDATED").await?;
    SearchIndexPendingDao::mark(tx, SearchEntity::Book, &[existing.id.as_str()]).await?;
    BookDao::update(tx, &updated).await
}

#[cfg(test)]
mod tests {
    use komga_scheduler::Task;
    use komga_types::BookMetadataPatchCapability::{Number, NumberSort, Title};

    use super::push_refresh;

    #[test]
    fn refreshes_of_one_book_join_their_capabilities() {
        let refresh = |book: &str, capabilities| {
            Task::refresh_book_metadata(book.into(), "S".into(), capabilities)
        };
        let mut tasks = vec![refresh("A", vec![Title])];
        push_refresh(&mut tasks, refresh("A", vec![Number, NumberSort]));
        push_refresh(&mut tasks, refresh("B", vec![Number, NumberSort]));
        assert_eq!(
            tasks,
            [
                refresh("A", vec![Title, Number, NumberSort]),
                refresh("B", vec![Number, NumberSort]),
            ]
        );
    }
}
