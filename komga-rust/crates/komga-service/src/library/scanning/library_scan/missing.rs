//! The deletion part of Kotlin `LibraryContentLifecycle.scanRootFolder`: soft deletes the series
//! and books a scan no longer finds, except under directories it could not list.

use std::collections::HashSet;

use komga_db::{DbError, DbTransaction};
use komga_types::{Book, FileUrl, Series, SeriesId};
use tracing::info;

use crate::library::scanning::file_system_scanner::Unread;
use crate::library::series;

/// Soft deletes the stored content missing from `scanned` and returns the live series that lost
/// books, in book order; Kotlin treats them as changed even if their directory time is not.
pub(super) async fn soft_delete<'a>(
    tx: &mut DbTransaction,
    stored_series: &'a [Series],
    stored_books: &[Book],
    scanned: &[(Series, Vec<Book>)],
    unread: &Unread,
) -> Result<Vec<&'a Series>, DbError> {
    let scanned_series: HashSet<&FileUrl> = scanned.iter().map(|(series, _)| &series.url).collect();
    // An empty scan re-deletes every series, already deleted ones included.
    let gone_series: Vec<&Series> = stored_series
        .iter()
        .filter(|series| {
            scanned.is_empty()
                || (series.deleted_date.is_none() && !scanned_series.contains(&series.url))
        })
        .filter(|series| !unread.contains(&series.url))
        .collect();
    if !gone_series.is_empty() {
        info!(
            count = gone_series.len(),
            "Soft deleting series not on disk anymore"
        );
        series::soft_delete_series(tx, &gone_series).await?;
    }

    let gone_series: HashSet<&SeriesId> = gone_series.iter().map(|series| &series.id).collect();
    let scanned_books: HashSet<&FileUrl> = scanned
        .iter()
        .flat_map(|(_, books)| books.iter().map(|book| &book.url))
        .collect();
    let gone_books: Vec<Book> = stored_books
        .iter()
        .filter(|book| {
            book.deleted_date.is_none()
                && !gone_series.contains(&book.series_id)
                && !scanned_books.contains(&book.url)
                && !unread.contains(&book.url)
        })
        .cloned()
        .collect();
    if gone_books.is_empty() {
        return Ok(Vec::new());
    }
    info!(
        count = gone_books.len(),
        "Soft deleting books not on disk anymore"
    );
    series::soft_delete_books(tx, &gone_books).await?;
    let mut changed: Vec<&Series> = Vec::new();
    for book in &gone_books {
        let Some(series) = stored_series
            .iter()
            .find(|series| series.id == book.series_id)
        else {
            continue;
        };
        if !changed.iter().any(|known| known.id == series.id) {
            changed.push(series);
        }
    }
    Ok(changed)
}
