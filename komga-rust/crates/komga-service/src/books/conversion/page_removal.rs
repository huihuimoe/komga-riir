//! Kotlin `BookPageEditor.removeHashedPages`: a CBZ book is rewritten without the pages whose
//! hash matched, keeping its other members and its ID. The rewritten file replaces the book file
//! once verified; until the database records it, a crash puts the original file back.

use std::collections::BTreeMap;
use std::path::Path;

use komga_db::PageHashDao;
use komga_scheduler::Task;
use komga_types::{
    BookId, BookPage, BookPageNumbered, LibraryId, Media, MediaStatus, PageHashAction,
};
use tracing::info;

use super::cbz::copy_zip;
use super::{BookConverter, ConversionError, Original, ZIP, book_path};
use crate::library::file_operation::SourceAfter;

/// Kotlin `BookPageEditor.convertibleTypes`.
const EDITABLE_TYPES: [&str; 1] = [ZIP];

/// Kotlin `removeDuplicatePages` of a map: one `RemoveHashedPages` per book, its pages in order.
pub(crate) fn removal_tasks(
    pages: impl IntoIterator<Item = (BookId, BookPageNumbered)>,
    priority: i32,
) -> Vec<Task> {
    let mut by_book: BTreeMap<BookId, Vec<BookPageNumbered>> = BTreeMap::new();
    for (book_id, page) in pages {
        by_book.entry(book_id).or_default().push(page);
    }
    by_book
        .into_iter()
        .map(|(book_id, pages)| Task::remove_hashed_pages(book_id, pages).with_priority(priority))
        .collect()
}

impl BookConverter {
    /// Kotlin `FindDuplicatePagesToDelete`: queues the removal of the library's pages whose hash
    /// is known to be deleted automatically, one task per book.
    pub(crate) async fn find_duplicate_pages_to_delete(
        &self,
        library_id: &LibraryId,
        priority: i32,
    ) -> Result<(), ConversionError> {
        let mut tx = self.db.begin().await?;
        let matches = PageHashDao::find_matches_by_known_action(
            &mut tx,
            PageHashAction::DeleteAuto,
            library_id,
        )
        .await?;
        tx.rollback().await?;
        let tasks = removal_tasks(matches, priority + 1);
        info!(library = %library_id, count = tasks.len(), "Found books with duplicate pages to delete");
        self.tasks.submit_all(&tasks).await?;
        Ok(())
    }

    /// Removes the `deleted` pages from the book; skipped unless each still is the book page at
    /// its number. Removing the first page queues a new book thumbnail.
    pub async fn remove_hashed_pages(
        &self,
        id: &BookId,
        deleted: &[BookPageNumbered],
        priority: i32,
    ) -> Result<(), ConversionError> {
        let Some((original, keep)) = self.editable(id, deleted).await? else {
            return Ok(());
        };
        let path = book_path(&original.book);
        info!(book = %id, count = deleted.len(), "Removing pages");
        let mut file = self
            .files
            .begin(&path, &path, Some(&path), SourceAfter::Keep)
            .await?;
        let mut names: Vec<String> = Vec::new();
        let files = original.media.files.iter().map(|file| &file.file_name);
        for name in keep.iter().map(|page| &page.file_name).chain(files) {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        let expected = Media {
            pages: keep,
            ..original.media.clone()
        };
        let write = move |source: &Path, target: &Path| copy_zip(source, target, &names);
        let mut follow_ups = Vec::new();
        if deleted.iter().any(|page| page.page_number == 1) {
            follow_ups.push(Task::generate_book_thumbnail(id.clone()).with_priority(priority + 1));
        }
        let result = self
            .write_and_record(&mut file, &original, &expected, write, deleted, &follow_ups)
            .await;
        self.files.finish(file).await?;
        if let Err(ConversionError::Unverified(_)) = &result {
            self.removal_failed
                .lock()
                .expect("Not poisoned")
                .insert(id.clone());
        }
        result?;
        Ok(())
    }

    /// The book with its media and the pages it keeps when the removal should go ahead; `None`
    /// when it is skipped.
    async fn editable(
        &self,
        id: &BookId,
        deleted: &[BookPageNumbered],
    ) -> Result<Option<(Original, Vec<BookPage>)>, ConversionError> {
        let Some((book, library, media)) =
            self.changeable(id, |_| true, &self.removal_failed).await?
        else {
            return Ok(None);
        };
        let media_type = media.media_type.clone().unwrap_or_default();
        if !EDITABLE_TYPES.contains(&media_type.as_str()) {
            return Err(ConversionError::MediaUnsupported(
                media_type,
                &EDITABLE_TYPES,
            ));
        }
        if media.status != MediaStatus::Ready {
            return Err(ConversionError::MediaNotReady);
        }
        let keep: Vec<BookPage> = (media.pages.iter().enumerate())
            .filter(|(index, page)| {
                !deleted.iter().any(|candidate| {
                    (
                        &candidate.file_hash,
                        &candidate.media_type,
                        &candidate.file_name,
                    ) == (&page.file_hash, &page.media_type, &page.file_name)
                        && usize::try_from(candidate.page_number) == Ok(index + 1)
                })
            })
            .map(|(_, page)| page.clone())
            .collect();
        if media.pages.len() != keep.len() + deleted.len() {
            info!(book = %id, "The pages to delete do not match the book pages, skipping");
            return Ok(None);
        }
        let original = Original {
            book,
            media,
            analyze_dimensions: library.analyze_dimensions,
        };
        Ok(Some((original, keep)))
    }
}
