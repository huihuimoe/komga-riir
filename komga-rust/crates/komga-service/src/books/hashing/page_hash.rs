//! Kotlin `PageHashLifecycle.getPage`/`createOrUpdate` and the `PageHashRepository` queries of
//! `PageHashController`: duplicate pages by hash.

use komga_db::{Database, DbError, PageHashDao};
use komga_scheduler::{DEFAULT_PRIORITY, Task, TaskQueue};
use komga_types::{
    BookId, BookPageNumbered, KnownPageHashSort, PageHashAction, PageHashKnown, PageHashMatch,
    PageHashMatchSort, PageHashUnknown, PageRequest, PageResult, SortOrder, UnknownPageHashSort,
};
use thiserror::Error;

use crate::books::conversion::removal_tasks;
use crate::reading::{PageContent, ReadingError, ReadingService};

/// Kotlin `createOrUpdate`: the longest side of the stored thumbnail, in pixels.
const KNOWN_THUMBNAIL_SIZE: u32 = 500;

#[derive(Debug, Error)]
pub enum PageHashError {
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error("Cannot read page: {0}")]
    Reading(#[from] ReadingError),
}

#[derive(Clone)]
pub struct PageHashService {
    db: Database,
    tasks: TaskQueue,
    reading: ReadingService,
}

impl PageHashService {
    #[must_use]
    pub fn new(db: Database, tasks: TaskQueue, reading: ReadingService) -> Self {
        Self { db, tasks, reading }
    }

    pub async fn known(
        &self,
        actions: Option<&[PageHashAction]>,
        sort: &[SortOrder<KnownPageHashSort>],
        page: PageRequest,
    ) -> Result<PageResult<PageHashKnown>, PageHashError> {
        Ok(PageHashDao::find_all_known(&self.db, actions, sort, page).await?)
    }

    pub async fn known_thumbnail(&self, hash: &str) -> Result<Option<Vec<u8>>, PageHashError> {
        Ok(PageHashDao::find_known_thumbnail(&self.db, hash).await?)
    }

    pub async fn unknown(
        &self,
        sort: &[SortOrder<UnknownPageHashSort>],
        page: PageRequest,
    ) -> Result<PageResult<PageHashUnknown>, PageHashError> {
        Ok(PageHashDao::find_all_unknown(&self.db, sort, page).await?)
    }

    pub async fn matches(
        &self,
        hash: &str,
        sort: &[SortOrder<PageHashMatchSort>],
        page: PageRequest,
    ) -> Result<PageResult<PageHashMatch>, PageHashError> {
        Ok(PageHashDao::find_matches_by_hash(&self.db, hash, sort, page).await?)
    }

    /// Kotlin `deleteDuplicatePagesByPageHash`: queues the removal of every page with `hash`, one
    /// task per book.
    pub async fn delete_all(&self, hash: &str) -> Result<(), PageHashError> {
        let matches = PageHashDao::find_matches_by_hash(&self.db, hash, &[], PageRequest::Unpaged)
            .await?
            .items;
        let pages = matches.into_iter().map(|found| {
            let page = BookPageNumbered {
                file_name: found.file_name,
                media_type: found.media_type,
                dimension: None,
                file_hash: hash.to_owned(),
                file_size: Some(found.file_size),
                page_number: found.page_number,
            };
            (found.book_id, page)
        });
        let tasks = removal_tasks(pages, DEFAULT_PRIORITY);
        Ok(self.tasks.submit_all(&tasks).await?)
    }

    /// Kotlin `deleteSingleMatchByPageHash`: queues the removal of the book `page`.
    pub async fn delete_match(
        &self,
        book_id: BookId,
        page: BookPageNumbered,
    ) -> Result<(), PageHashError> {
        Ok(self
            .tasks
            .submit(&Task::remove_hashed_pages(book_id, vec![page]))
            .await?)
    }

    /// Kotlin `getPage`: the first page with `hash`, resized to `resize` pixels on its longest
    /// side when given; `None` when no page has the hash.
    pub async fn page(
        &self,
        hash: &str,
        resize: Option<u32>,
    ) -> Result<Option<PageContent>, PageHashError> {
        let first = PageRequest::Paged { number: 0, size: 1 };
        let matches = PageHashDao::find_matches_by_hash(&self.db, hash, &[], first).await?;
        let Some(page) = matches.items.into_iter().next() else {
            return Ok(None);
        };
        match self
            .reading
            .read_book_page(&page.book_id, page.page_number, resize)
            .await
        {
            Ok(content) => Ok(Some(content)),
            Err(ReadingError::NotFound) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Kotlin `createOrUpdate`: a new hash is stored with a thumbnail of its first page, if any;
    /// a known hash only changes action.
    pub async fn create_or_update(
        &self,
        hash: &str,
        size: Option<i64>,
        action: PageHashAction,
    ) -> Result<(), PageHashError> {
        let mut tx = self.db.begin().await?;
        let exists = PageHashDao::exists_known(&mut tx, hash).await?;
        tx.rollback().await?;
        let thumbnail = if exists {
            None
        } else {
            self.page(hash, Some(KNOWN_THUMBNAIL_SIZE)).await?
        };

        let mut tx = self.db.begin_immediate().await?;
        // Another request may have stored the hash while the thumbnail was read.
        if PageHashDao::exists_known(&mut tx, hash).await? {
            PageHashDao::update_action(&mut tx, hash, action).await?;
        } else {
            let thumbnail = thumbnail.as_ref().map(|content| content.bytes.as_slice());
            PageHashDao::insert(&mut tx, hash, size, action, thumbnail).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
