//! Kotlin `tryRestoreSeries` and `tryRestoreBooks`: a new book with the size and file hash of a
//! soft-deleted book takes over its state, and a new series whose books all match the books of a
//! soft-deleted series takes over that series.

use std::collections::HashMap;

use komga_db::{BookDao, DbError, DbTransaction, RestoreDao, SeriesDao};
use komga_scheduler::Task;
use komga_types::{
    Book, BookMetadataPatchCapability, BookRef, DomainEvent, FileUrl, Series, SeriesId,
};
use tracing::info;

use crate::library::series;

/// The file hashes a reconciliation knows, by book URL, and the books it still needs hashed.
pub(super) struct Hashes<'a> {
    pub(super) known: &'a HashMap<FileUrl, String>,
    pub(super) missing: Vec<Book>,
}

impl<'a> Hashes<'a> {
    /// The file hash of `book`; when unknown, `book` is recorded as missing.
    pub(super) fn get(&mut self, book: &Book) -> Option<&'a str> {
        let hash = self.known.get(&book.url).map(String::as_str);
        if hash.is_none() {
            self.missing.push(book.clone());
        }
        hash
    }
}

/// Restores `books`, just added to `series`, from soft-deleted books; a `new_series` may also
/// take over a soft-deleted series. Books whose hash is missing from `hashes` leave everything
/// as it was. Returns the title refreshes of restored books.
pub(super) async fn try_restore(
    tx: &mut DbTransaction,
    series: &Series,
    books: &[Book],
    new_series: bool,
    hashes: &mut Hashes<'_>,
) -> Result<Vec<Task>, DbError> {
    let sizes: Vec<i64> = books.iter().map(|book| book.file_size).collect();
    if sizes.is_empty() {
        return Ok(Vec::new());
    }
    let mut candidates = BookDao::find_all_deleted_hashed_by_sizes(tx, &sizes).await?;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let deleted_series = if new_series {
        deleted_series_with_sizes(tx, sizes, &candidates).await?
    } else {
        Vec::new()
    };
    let mut hashed = Vec::new();
    let mut complete = true;
    for book in books {
        if deleted_series.is_empty()
            && !candidates
                .iter()
                .any(|candidate| candidate.file_size == book.file_size)
        {
            continue;
        }
        match hashes.get(book) {
            Some(hash) => hashed.push((book, hash)),
            None => complete = false,
        }
    }
    if !complete {
        return Ok(Vec::new());
    }

    for (book, hash) in &hashed {
        BookDao::update_file_hash(tx, &book.id, hash).await?;
    }
    let mut new_hashes: Vec<&str> = hashed.iter().map(|(_, hash)| *hash).collect();
    new_hashes.sort_unstable();
    let matched_series = deleted_series
        .into_iter()
        .find(|(_, deleted_hashes)| *deleted_hashes == new_hashes)
        .map(|(id, _)| id);
    // The books of the matched series go first, so that it hands over their state.
    if let Some(matched) = &matched_series {
        candidates.sort_by_key(|candidate| candidate.series_id != *matched);
    }

    let mut refreshes = Vec::new();
    for (book, hash) in hashed {
        let Some(index) = candidates.iter().position(|candidate| {
            candidate.file_size == book.file_size && candidate.file_hash == hash
        }) else {
            continue;
        };
        let deleted = candidates.remove(index);
        info!(book = %book.url, deleted = %deleted.url, "Restore deleted book");
        let kept_title = RestoreDao::restore_book(tx, &deleted.id, &book.id).await?;
        tx.record(DomainEvent::BookDeleted(BookRef::from(&deleted)));
        if kept_title {
            refreshes.push(Task::refresh_book_metadata(
                book.id.clone(),
                series.id.clone(),
                vec![BookMetadataPatchCapability::Title],
            ));
        }
    }
    if let Some(deleted) = matched_series {
        info!(series = %series.url, deleted = %deleted, "Restore deleted series");
        let removed = RestoreDao::restore_series(tx, &deleted, &series.id).await?;
        series::record_deleted(tx, removed);
    }
    Ok(refreshes)
}

/// The soft-deleted series of `candidates` whose books are all hashed and have exactly the
/// `sizes` of the new books, with their sorted book hashes.
async fn deleted_series_with_sizes(
    tx: &mut DbTransaction,
    mut sizes: Vec<i64>,
    candidates: &[Book],
) -> Result<Vec<(SeriesId, Vec<String>)>, DbError> {
    sizes.sort_unstable();
    let mut ids: Vec<&SeriesId> = candidates.iter().map(|book| &book.series_id).collect();
    ids.sort_unstable_by_key(|id| id.as_str());
    ids.dedup();
    let mut matches = Vec::new();
    for id in ids {
        let deleted = SeriesDao::find_by_id(tx, id)
            .await?
            .is_some_and(|series| series.deleted_date.is_some());
        if !deleted {
            continue;
        }
        let books = BookDao::find_all_by_series(tx, id).await?;
        let mut book_sizes: Vec<i64> = books.iter().map(|book| book.file_size).collect();
        book_sizes.sort_unstable();
        if book_sizes == sizes && books.iter().all(|book| !book.file_hash.is_empty()) {
            let mut hashes: Vec<String> = books.into_iter().map(|book| book.file_hash).collect();
            hashes.sort_unstable();
            matches.push((id.clone(), hashes));
        }
    }
    Ok(matches)
}
