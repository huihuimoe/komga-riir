//! Kotlin `LocalArtworkLifecycle.refreshLocalArtwork`, adding sidecar thumbnails through
//! `BookLifecycle.addThumbnailForBook` and `SeriesLifecycle.addThumbnailForSeries`.
//!
//! The artwork bytes are stored next to their URL, so serving them does not read the library. A
//! refresh replaces all sidecar thumbnails of its book or series with what is on disk now, so that
//! a file that stopped being artwork is not served from its earlier read.

use komga_db::{
    BookDao, DbError, DbTransaction, LibraryDao, NewThumbnail, SeriesDao, ThumbnailBookDao,
    ThumbnailSeriesDao,
};
use komga_types::{
    BookId, BookRef, DomainEvent, LibraryId, SeriesId, SeriesRef, ThumbnailId, ThumbnailRef,
    ThumbnailType,
};
use tokio::task;
use tracing::{debug, info, warn};

use super::local_files::{self, LocalArtwork};
use super::{ArtworkError, ArtworkService, house_keeping};

impl ArtworkService {
    pub(crate) async fn refresh_book_local_artwork(&self, id: &BookId) -> Result<(), ArtworkError> {
        let mut tx = self.db.begin().await?;
        let Some(book) = BookDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            warn!(book = %id, "Cannot refresh local artwork: Book does not exist");
            return Ok(());
        };
        let import = imports_local_artwork(&mut tx, &book.library_id).await?;
        tx.rollback().await?;
        info!(book = %id, url = %book.url, "Refresh local artwork for book");
        if !import {
            info!("Library is not set to import local artwork, skipping");
            return Ok(());
        }
        let path = book
            .url
            .to_path()
            .ok_or_else(|| ArtworkError::InvalidUrl(book.url.clone()))?;
        let candidates =
            task::spawn_blocking(move || local_files::book_candidates(&path)).await??;
        let found = local_files::load(candidates).await?;

        let mut tx = self.db.begin_immediate().await?;
        let Some(book) = BookDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            return Ok(());
        };
        ThumbnailBookDao::delete_by_book_and_type(&mut tx, id, ThumbnailType::Sidecar).await?;
        let mut first = None;
        for artwork in &found {
            let thumbnail_id = ThumbnailId::generate();
            ThumbnailBookDao::insert(&mut tx, id, &new_sidecar(&thumbnail_id, artwork)).await?;
            first.get_or_insert(thumbnail_id);
        }
        // `IF_NONE_OR_GENERATED` for the first file, `NO` for the others.
        let selected = ThumbnailBookDao::find_selected(&mut tx, id).await?;
        let select_first =
            selected.is_none_or(|selected| selected.kind == ThumbnailType::Generated);
        match first {
            Some(first) if select_first => {
                ThumbnailBookDao::mark_selected(&mut tx, id, &first).await?;
            }
            _ => house_keeping::book(&mut tx, id).await?,
        }
        for index in 0..found.len() {
            tx.record(DomainEvent::ThumbnailBookAdded(ThumbnailRef {
                owner: BookRef::from(&book),
                selected: index == 0 && select_first,
            }));
        }
        self.events.commit(tx).await?;
        Ok(())
    }

    pub(crate) async fn refresh_series_local_artwork(
        &self,
        id: &SeriesId,
    ) -> Result<(), ArtworkError> {
        let mut tx = self.db.begin().await?;
        let Some(series) = SeriesDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            warn!(series = %id, "Cannot refresh local artwork: Series does not exist");
            return Ok(());
        };
        let import = imports_local_artwork(&mut tx, &series.library_id).await?;
        tx.rollback().await?;
        info!(series = %id, url = %series.url, "Refresh local artwork for series");
        if !import {
            info!("Library is not set to import local artwork, skipping");
            return Ok(());
        }
        if series.oneshot {
            debug!("Disabled for oneshot series, skipping");
            return Ok(());
        }
        let path = series
            .url
            .to_path()
            .ok_or_else(|| ArtworkError::InvalidUrl(series.url.clone()))?;
        let candidates =
            task::spawn_blocking(move || local_files::series_candidates(&path)).await??;
        let found = local_files::load(candidates).await?;

        let mut tx = self.db.begin_immediate().await?;
        let Some(series) = SeriesDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            return Ok(());
        };
        ThumbnailSeriesDao::delete_by_series_and_type(&mut tx, id, ThumbnailType::Sidecar).await?;
        let mut first = None;
        for artwork in &found {
            let thumbnail_id = ThumbnailId::generate();
            ThumbnailSeriesDao::insert(&mut tx, id, &new_sidecar(&thumbnail_id, artwork)).await?;
            first.get_or_insert(thumbnail_id);
        }
        // Series have no generated thumbnail: `IF_NONE_OR_GENERATED` selects only when none is.
        let entries = ThumbnailSeriesDao::find_all_by_series(&mut tx, id).await?;
        let select_first = !entries.iter().any(|entry| entry.selected);
        match first {
            Some(first) if select_first => {
                ThumbnailSeriesDao::mark_selected(&mut tx, id, &first).await?;
            }
            _ => house_keeping::series(&mut tx, id).await?,
        }
        for index in 0..found.len() {
            tx.record(DomainEvent::ThumbnailSeriesAdded(ThumbnailRef {
                owner: SeriesRef::from(&series),
                selected: index == 0 && select_first,
            }));
        }
        self.events.commit(tx).await?;
        Ok(())
    }
}

async fn imports_local_artwork(
    tx: &mut DbTransaction,
    library_id: &LibraryId,
) -> Result<bool, DbError> {
    let library = LibraryDao::find_by_id(tx, library_id)
        .await?
        .expect("The library of a stored book or series exists");
    Ok(library.import_local_artwork)
}

fn new_sidecar<'a>(id: &'a ThumbnailId, artwork: &'a LocalArtwork) -> NewThumbnail<'a> {
    NewThumbnail {
        id,
        thumbnail: Some(&artwork.bytes),
        url: Some(&artwork.url),
        kind: ThumbnailType::Sidecar,
        media_type: artwork.media_type,
        file_size: i64::try_from(artwork.bytes.len()).expect("Artwork size is bounded"),
        dimension: artwork.dimension,
    }
}
