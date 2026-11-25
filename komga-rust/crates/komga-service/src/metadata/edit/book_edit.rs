//! Kotlin `BookController.updateBookMetadata` and `updateBookMetadataByBatch`: an admin's edit of
//! book metadata, including its locks.

use jiff::civil::Date;
use komga_db::{BookDao, BookMetadataDao, SearchIndexPendingDao};
use komga_scheduler::{Task, TaskQueue};
use komga_types::{
    BookAuthor, BookId, BookMetadata, BookRef, DomainEvent, KomgaUser, NotableEntity, SearchEntity,
    WebLink,
};

use super::super::manual_refresh::require_admin;
use super::super::{MetadataError, MetadataService};

/// The resolved edit: each `Some` replaces the stored value, `None` keeps it. A request that
/// unsets a field gives its empty value here.
#[derive(Debug, Clone, Default)]
pub struct BookMetadataUpdate {
    pub title: Option<String>,
    pub title_lock: Option<bool>,
    pub summary: Option<String>,
    pub summary_lock: Option<bool>,
    pub number: Option<String>,
    pub number_lock: Option<bool>,
    pub number_sort: Option<f32>,
    pub number_sort_lock: Option<bool>,
    pub release_date: Option<Option<Date>>,
    pub release_date_lock: Option<bool>,
    pub authors: Option<Vec<BookAuthor>>,
    pub authors_lock: Option<bool>,
    pub notable_entities: Option<Vec<NotableEntity>>,
    pub notable_entities_lock: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub tags_lock: Option<bool>,
    pub isbn: Option<String>,
    pub isbn_lock: Option<bool>,
    pub links: Option<Vec<WebLink>>,
    pub links_lock: Option<bool>,
}

impl BookMetadataUpdate {
    fn apply(self, m: BookMetadata) -> BookMetadata {
        BookMetadata {
            book_id: m.book_id,
            title: self.title.unwrap_or(m.title),
            title_lock: self.title_lock.unwrap_or(m.title_lock),
            summary: self.summary.unwrap_or(m.summary),
            summary_lock: self.summary_lock.unwrap_or(m.summary_lock),
            number: self.number.unwrap_or(m.number),
            number_lock: self.number_lock.unwrap_or(m.number_lock),
            number_sort: self.number_sort.unwrap_or(m.number_sort),
            number_sort_lock: self.number_sort_lock.unwrap_or(m.number_sort_lock),
            release_date: self.release_date.unwrap_or(m.release_date),
            release_date_lock: self.release_date_lock.unwrap_or(m.release_date_lock),
            authors: self.authors.unwrap_or(m.authors),
            authors_lock: self.authors_lock.unwrap_or(m.authors_lock),
            notable_entities: self.notable_entities.unwrap_or(m.notable_entities),
            notable_entities_lock: self
                .notable_entities_lock
                .unwrap_or(m.notable_entities_lock),
            tags: self.tags.unwrap_or(m.tags),
            tags_lock: self.tags_lock.unwrap_or(m.tags_lock),
            isbn: self.isbn.unwrap_or(m.isbn),
            isbn_lock: self.isbn_lock.unwrap_or(m.isbn_lock),
            links: self.links.unwrap_or(m.links),
            links_lock: self.links_lock.unwrap_or(m.links_lock),
        }
        .normalized()
    }
}

impl MetadataService {
    pub async fn update_book_metadata(
        &self,
        user: &KomgaUser,
        id: &BookId,
        update: BookMetadataUpdate,
    ) -> Result<(), MetadataError> {
        if self.update_all(user, vec![(id.clone(), update)]).await? {
            Ok(())
        } else {
            Err(MetadataError::NotFound)
        }
    }

    /// Edits the existing books among `updates` and skips the others.
    pub async fn update_books_metadata(
        &self,
        user: &KomgaUser,
        updates: Vec<(BookId, BookMetadataUpdate)>,
    ) -> Result<(), MetadataError> {
        self.update_all(user, updates).await?;
        Ok(())
    }

    /// Applies the edits of the existing books in one transaction, then queues the aggregation of
    /// each changed series. Returns whether a book was edited.
    async fn update_all(
        &self,
        user: &KomgaUser,
        updates: Vec<(BookId, BookMetadataUpdate)>,
    ) -> Result<bool, MetadataError> {
        require_admin(user)?;
        let mut tx = self.db.begin_immediate().await?;
        let mut aggregations: Vec<Task> = Vec::new();
        for (id, update) in updates {
            let Some(metadata) = BookMetadataDao::find_by_id(&mut tx, &id).await? else {
                continue;
            };
            BookMetadataDao::update(&mut tx, &update.apply(metadata)).await?;
            SearchIndexPendingDao::mark(&mut tx, SearchEntity::Book, &[id.as_str()]).await?;
            let book = BookDao::find_by_id(&mut tx, &id)
                .await?
                .expect("Book metadata belongs to a book (foreign key)");
            tx.record(DomainEvent::BookUpdated(BookRef::from(&book)));
            let aggregation = Task::aggregate_series_metadata(book.series_id);
            if !aggregations.contains(&aggregation) {
                aggregations.push(aggregation);
            }
        }
        TaskQueue::stage(&mut tx, &aggregations).await?;
        self.events.commit(tx).await?;
        self.search.notify();
        self.tasks.hand_over(&self.db).await;
        Ok(!aggregations.is_empty())
    }
}
