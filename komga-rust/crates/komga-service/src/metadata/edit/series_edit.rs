//! Kotlin `SeriesController.updateSeriesMetadata`: an admin's edit of series metadata, including
//! its locks.

use komga_db::{SearchIndexPendingDao, SeriesDao, SeriesMetadataDao};
use komga_types::{
    AlternateTitle, DomainEvent, KomgaUser, ReadingDirection, SeriesId, SeriesMetadata, SeriesRef,
    SeriesStatus, WebLink,
};

use super::super::manual_refresh::require_admin;
use super::super::{MetadataError, MetadataService};

/// The resolved edit: each `Some` replaces the stored value, `None` keeps it. A request that
/// unsets a field gives its empty value here.
#[derive(Debug, Clone, Default)]
pub struct SeriesMetadataUpdate {
    pub status: Option<SeriesStatus>,
    pub status_lock: Option<bool>,
    pub title: Option<String>,
    pub title_lock: Option<bool>,
    pub title_sort: Option<String>,
    pub title_sort_lock: Option<bool>,
    pub summary: Option<String>,
    pub summary_lock: Option<bool>,
    pub reading_direction: Option<Option<ReadingDirection>>,
    pub reading_direction_lock: Option<bool>,
    pub publisher: Option<String>,
    pub publisher_lock: Option<bool>,
    pub age_rating: Option<Option<i32>>,
    pub age_rating_lock: Option<bool>,
    /// A normalized BCP 47 tag, or empty.
    pub language: Option<String>,
    pub language_lock: Option<bool>,
    pub genres: Option<Vec<String>>,
    pub genres_lock: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub tags_lock: Option<bool>,
    pub total_book_count: Option<Option<i32>>,
    pub total_book_count_lock: Option<bool>,
    pub sharing_labels: Option<Vec<String>>,
    pub sharing_labels_lock: Option<bool>,
    pub links: Option<Vec<WebLink>>,
    pub links_lock: Option<bool>,
    pub alternate_titles: Option<Vec<AlternateTitle>>,
    pub alternate_titles_lock: Option<bool>,
}

impl SeriesMetadataUpdate {
    fn apply(self, m: SeriesMetadata) -> SeriesMetadata {
        SeriesMetadata {
            series_id: m.series_id,
            status: self.status.unwrap_or(m.status),
            status_lock: self.status_lock.unwrap_or(m.status_lock),
            title: self.title.unwrap_or(m.title),
            title_lock: self.title_lock.unwrap_or(m.title_lock),
            title_sort: self.title_sort.unwrap_or(m.title_sort),
            title_sort_lock: self.title_sort_lock.unwrap_or(m.title_sort_lock),
            summary: self.summary.unwrap_or(m.summary),
            summary_lock: self.summary_lock.unwrap_or(m.summary_lock),
            reading_direction: self.reading_direction.unwrap_or(m.reading_direction),
            reading_direction_lock: self
                .reading_direction_lock
                .unwrap_or(m.reading_direction_lock),
            publisher: self.publisher.unwrap_or(m.publisher),
            publisher_lock: self.publisher_lock.unwrap_or(m.publisher_lock),
            age_rating: self.age_rating.unwrap_or(m.age_rating),
            age_rating_lock: self.age_rating_lock.unwrap_or(m.age_rating_lock),
            language: self.language.unwrap_or(m.language),
            language_lock: self.language_lock.unwrap_or(m.language_lock),
            genres: self.genres.unwrap_or(m.genres),
            genres_lock: self.genres_lock.unwrap_or(m.genres_lock),
            tags: self.tags.unwrap_or(m.tags),
            tags_lock: self.tags_lock.unwrap_or(m.tags_lock),
            total_book_count: self.total_book_count.unwrap_or(m.total_book_count),
            total_book_count_lock: self
                .total_book_count_lock
                .unwrap_or(m.total_book_count_lock),
            sharing_labels: self.sharing_labels.unwrap_or(m.sharing_labels),
            sharing_labels_lock: self.sharing_labels_lock.unwrap_or(m.sharing_labels_lock),
            links: self.links.unwrap_or(m.links),
            links_lock: self.links_lock.unwrap_or(m.links_lock),
            alternate_titles: self.alternate_titles.unwrap_or(m.alternate_titles),
            alternate_titles_lock: self
                .alternate_titles_lock
                .unwrap_or(m.alternate_titles_lock),
        }
        .normalized()
    }
}

impl MetadataService {
    pub async fn update_series_metadata(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
        update: SeriesMetadataUpdate,
    ) -> Result<(), MetadataError> {
        require_admin(user)?;
        let mut tx = self.db.begin_immediate().await?;
        let Some(metadata) = SeriesMetadataDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            return Err(MetadataError::NotFound);
        };
        SeriesMetadataDao::update(&mut tx, &update.apply(metadata)).await?;
        SearchIndexPendingDao::mark_series_metadata(&mut tx, id).await?;
        let series = SeriesDao::find_by_id(&mut tx, id)
            .await?
            .expect("Series metadata belongs to a series (foreign key)");
        tx.record(DomainEvent::SeriesUpdated(SeriesRef::from(&series)));
        self.events.commit(tx).await?;
        self.search.notify();
        Ok(())
    }
}
