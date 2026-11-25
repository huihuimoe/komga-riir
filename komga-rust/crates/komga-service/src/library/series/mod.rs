//! Series organization: `SeriesLifecycle`.
//!
//! Kotlin `SeriesLifecycle.{createSeries,addBooks,softDeleteMany,sortBooks}` inside the caller's
//! transaction, recording their events in it. The series and books whose searchable state changes
//! are marked for the search index.

mod book_order;

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use komga_db::{
    BookDao, BookMetadataAggregationDao, BookMetadataDao, BookNumbering, DbError, DbTransaction,
    DeletedContent, MediaDao, SearchIndexPendingDao, SeriesDao, SeriesMetadataDao,
};
use komga_scheduler::Task;
use komga_types::{
    Book, BookId, BookMetadataPatchCapability, DomainEvent, SearchEntity, Series, SeriesId,
    SeriesRef,
};
use thiserror::Error;
use tracing::info;

use crate::support::natural_order;

#[derive(Debug, Error)]
pub enum SeriesError {
    /// Kotlin `sortBooks` fails the same way on a book without a metadata row.
    #[error("Book {0} has no metadata")]
    MissingBookMetadata(BookId),
    #[error("Database error: {0}")]
    Db(#[from] DbError),
}

/// `createSeries`: the series with its default metadata and an empty aggregation.
pub(crate) async fn create_series(tx: &mut DbTransaction, series: &Series) -> Result<(), DbError> {
    SeriesDao::insert(tx, series).await?;
    SeriesMetadataDao::insert_for_new_series(tx, series).await?;
    BookMetadataAggregationDao::insert_empty(tx, &series.id).await?;
    SearchIndexPendingDao::mark(tx, SearchEntity::Series, &[series.id.as_str()]).await?;
    tx.record(DomainEvent::SeriesAdded(series.into()));
    Ok(())
}

/// `addBooks`: the books with an `UNKNOWN` media and default metadata each.
pub(crate) async fn add_books(
    tx: &mut DbTransaction,
    series: &Series,
    books: &[Book],
) -> Result<(), DbError> {
    let books: Vec<Book> = books
        .iter()
        .map(|book| Book {
            series_id: series.id.clone(),
            ..book.clone()
        })
        .collect();
    BookDao::insert(tx, &books).await?;
    let ids: Vec<BookId> = books.iter().map(|book| book.id.clone()).collect();
    MediaDao::insert_unknown(tx, &ids).await?;
    BookMetadataDao::insert_for_new_books(tx, &books).await?;
    let ids: Vec<&str> = ids.iter().map(BookId::as_str).collect();
    SearchIndexPendingDao::mark(tx, SearchEntity::Book, &ids).await?;
    for book in &books {
        tx.record(DomainEvent::BookAdded(book.into()));
    }
    Ok(())
}

/// `softDeleteMany`: soft deletes the series with all their books, re-dating books deleted before.
pub(crate) async fn soft_delete_series(
    tx: &mut DbTransaction,
    series: &[&Series],
) -> Result<(), DbError> {
    let deleted_date = local_now();
    for series in series {
        let books = BookDao::find_all_by_series(tx, &series.id).await?;
        soft_delete_books(tx, &books).await?;
        SeriesDao::update_file_state(
            tx,
            &series.id,
            series.file_last_modified,
            Some(deleted_date),
        )
        .await?;
        tx.record(DomainEvent::SeriesUpdated(SeriesRef::from(*series)));
    }
    let ids: Vec<&str> = series.iter().map(|series| series.id.as_str()).collect();
    SearchIndexPendingDao::mark(tx, SearchEntity::Series, &ids).await
}

/// Kotlin `BookLifecycle.softDeleteMany`.
pub(crate) async fn soft_delete_books(
    tx: &mut DbTransaction,
    books: &[Book],
) -> Result<(), DbError> {
    let deleted_date = local_now();
    for book in books {
        let deleted = Book {
            deleted_date: Some(deleted_date),
            ..book.clone()
        };
        BookDao::update(tx, &deleted).await?;
        tx.record(DomainEvent::BookUpdated(book.into()));
    }
    let ids: Vec<&str> = books.iter().map(|book| book.id.as_str()).collect();
    SearchIndexPendingDao::mark(tx, SearchEntity::Book, &ids).await
}

/// Records the events of a hard deletion: `BookLifecycle.deleteMany`, then
/// `SeriesLifecycle.deleteMany`.
pub(crate) fn record_deleted(tx: &mut DbTransaction, deleted: DeletedContent) {
    for book in deleted.books {
        tx.record(DomainEvent::BookDeleted(book));
    }
    for series in deleted.series {
        tx.record(DomainEvent::SeriesDeleted(series));
    }
}

/// Kotlin `LocalDateTime.now()`.
fn local_now() -> DateTime {
    Timestamp::now()
        .to_zoned_datetime(TimeZone::system())
        .datetime()
}

/// `sortBooks`: numbers every book of the series, soft-deleted ones included, in name order and
/// stores the book count, which the series' search document holds. Unlocked metadata numbers
/// follow; returns the metadata refreshes Kotlin submits for books whose metadata numbering
/// changed, to submit after the commit.
pub(crate) async fn sort_books(
    tx: &mut DbTransaction,
    series_id: &SeriesId,
) -> Result<Vec<Task>, SeriesError> {
    info!(series = %series_id, "Sorting books");
    let mut books = BookDao::find_all_by_series(tx, series_id).await?;
    let ids: Vec<BookId> = books.iter().map(|book| book.id.clone()).collect();
    let numberings = BookMetadataDao::find_numbering(tx, &ids).await?;
    let mut keyed: Vec<(String, usize)> = books
        .iter()
        .enumerate()
        .map(|(index, book)| (book_order::sort_key(&book.name), index))
        .collect();
    keyed.sort_by(|(left, _), (right, _)| natural_order::compare(left, right));

    let mut refreshes = Vec::new();
    for (position, (_, index)) in keyed.into_iter().enumerate() {
        let number = i32::try_from(position + 1).expect("A series has fewer than 2^31 books");
        let book = &mut books[index];
        book.number = number;
        BookDao::update(tx, book).await?;
        let old = numberings
            .iter()
            .find(|numbering| numbering.book_id == book.id)
            .ok_or_else(|| SeriesError::MissingBookMetadata(book.id.clone()))?;
        if old.number_lock && old.number_sort_lock {
            continue;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "Kotlin `Int.toFloat()` rounds the same way"
        )]
        let new = BookNumbering {
            number: if old.number_lock {
                old.number.clone()
            } else {
                number.to_string()
            },
            number_sort: if old.number_sort_lock {
                old.number_sort
            } else {
                number as f32
            },
            ..old.clone()
        };
        BookMetadataDao::update_numbering(tx, &new).await?;
        // Refresh metadata to reimport the book number; resorting would otherwise overwrite it.
        if old.number != new.number || old.number_sort != new.number_sort {
            refreshes.push(Task::refresh_book_metadata(
                book.id.clone(),
                series_id.clone(),
                vec![
                    BookMetadataPatchCapability::Number,
                    BookMetadataPatchCapability::NumberSort,
                ],
            ));
        }
    }
    let count = i32::try_from(books.len()).expect("A series has fewer than 2^31 books");
    SeriesDao::update_book_count(tx, series_id, count).await?;
    SearchIndexPendingDao::mark(tx, SearchEntity::Series, &[series_id.as_str()]).await?;
    Ok(refreshes)
}
