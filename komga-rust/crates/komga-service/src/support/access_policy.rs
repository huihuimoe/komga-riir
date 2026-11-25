//! Kotlin `ContentRestrictionChecker`: whether a user may see one item.

use komga_db::{BookDao, DbError, DbTransaction, SeriesDao, SeriesMetadataDao};
use komga_types::{Book, BookId, KomgaUser, LibraryId, Series, SeriesId};

/// The outcome of a check by ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    Allowed,
    Denied,
    /// The item is unknown to a check that had to look it up.
    Unknown,
}

/// `checkContentRestrictionBook(user, bookId)`: only a user limited to some libraries or some
/// content looks the book up, so an unknown book is [`Access::Unknown`] for them alone.
pub(crate) async fn check_book_id(
    transaction: &mut DbTransaction,
    user: &KomgaUser,
    book_id: &BookId,
) -> Result<Access, DbError> {
    if is_unlimited(user) {
        return Ok(Access::Allowed);
    }
    let Some(book) = BookDao::find_by_id(transaction, book_id).await? else {
        return Ok(Access::Unknown);
    };
    Ok(if can_access_book(transaction, user, &book).await? {
        Access::Allowed
    } else {
        Access::Denied
    })
}

/// `checkContentRestrictionSeries(user, seriesId)`, looking the series up like
/// [`check_book_id`]. Kotlin reads a restricted user's unknown series from the metadata table and
/// fails there; it is [`Access::Unknown`] here, as for books.
pub(crate) async fn check_series_id(
    transaction: &mut DbTransaction,
    user: &KomgaUser,
    series_id: &SeriesId,
) -> Result<Access, DbError> {
    if is_unlimited(user) {
        return Ok(Access::Allowed);
    }
    let Some(series) = SeriesDao::find_by_id(transaction, series_id).await? else {
        return Ok(Access::Unknown);
    };
    Ok(if can_access_series(transaction, user, &series).await? {
        Access::Allowed
    } else {
        Access::Denied
    })
}

/// Whether a user sees every library without content restrictions, so that checks by ID need not
/// look the item up.
fn is_unlimited(user: &KomgaUser) -> bool {
    (user.shared_all_libraries || user.is_admin()) && !user.restrictions.is_restricted()
}

/// `checkContentRestrictionBook(user, book)`.
pub(crate) async fn can_access_book(
    transaction: &mut DbTransaction,
    user: &KomgaUser,
    book: &Book,
) -> Result<bool, DbError> {
    can_access_content(transaction, user, &book.library_id, &book.series_id).await
}

/// `checkContentRestrictionSeries(user, seriesId)` of an existing series.
pub(crate) async fn can_access_series(
    transaction: &mut DbTransaction,
    user: &KomgaUser,
    series: &Series,
) -> Result<bool, DbError> {
    can_access_content(transaction, user, &series.library_id, &series.id).await
}

/// The library must be shared with the user, and a restricted user must be allowed the series'
/// age rating and sharing labels.
async fn can_access_content(
    transaction: &mut DbTransaction,
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
    let (age_rating, sharing_labels) =
        SeriesMetadataDao::find_age_rating_and_sharing_labels(transaction, series_id).await?;
    Ok(user.is_content_allowed(age_rating, &sharing_labels))
}
