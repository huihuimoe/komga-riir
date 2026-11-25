//! Kotlin `TransientBookLifecycle.getMetadata`: what the metadata providers say about a book that
//! is in no library.

use komga_db::{Database, DbError, SeriesMetadataDao};
use komga_types::{Book, BookMetadataPatchCapability, Media, SeriesId};

use super::providers::{BOOK_PROVIDERS, SERIES_FROM_BOOK_PROVIDERS, SeriesFromBookProvider};
use crate::reading::ReadingService;

/// The first number sort a book provider finds, and the series titled as a provider names the
/// book's series. Library settings are not consulted, as in Kotlin.
pub(crate) async fn number_and_series(
    db: &Database,
    reading: &ReadingService,
    book: &Book,
    media: &Media,
) -> Result<(Option<f32>, Option<SeriesId>), DbError> {
    let mut number = None;
    for provider in BOOK_PROVIDERS {
        // Providers without the capability never set it, so they are not asked.
        if !provider
            .capabilities()
            .contains(&BookMetadataPatchCapability::NumberSort)
        {
            continue;
        }
        number = provider
            .book_patch(reading, book, media)
            .await
            .and_then(|patch| patch.number_sort);
        if number.is_some() {
            break;
        }
    }
    let mut titles = Vec::new();
    for provider in SERIES_FROM_BOOK_PROVIDERS {
        let append_volume = match provider {
            SeriesFromBookProvider::ComicInfo => &[true, false][..],
            SeriesFromBookProvider::Epub => &[false],
        };
        for &append in append_volume {
            let title = provider
                .series_patch(book, media, append)
                .await
                .and_then(|patch| patch.title);
            titles.extend(title.filter(|title| !title.trim().is_empty()));
        }
    }
    let series = if titles.is_empty() {
        None
    } else {
        let mut tx = db.begin().await?;
        let series = SeriesMetadataDao::find_series_id_by_titles(&mut tx, &titles).await?;
        tx.rollback().await?;
        series
    };
    Ok((number, series))
}
