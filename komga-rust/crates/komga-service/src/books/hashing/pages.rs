//! Page hashing: Kotlin `PageHashLifecycle.getBookIdsWithMissingPageHash`,
//! `BookLifecycle.hashPagesAndPersist`, and `BookAnalyzer.hashPages`/`hashPage`.

use std::path::{Path, PathBuf};

use komga_archive::digest::FileDigest;
use komga_archive::divina::DivinaFormat;
use komga_db::{BookDao, LibraryDao, MediaDao};
use komga_image as image;
use komga_scheduler::Task;
use komga_types::{BookId, BookPage, LibraryId, Media, MediaStatus};
use tokio::task;
use tracing::{info, warn};

use super::{HashingError, HashingService, is_recorded};

/// `PageHashLifecycle.hashableMediaTypes`.
const HASHABLE_MEDIA_TYPES: &[&str] = &["application/zip"];
const JPEG: &str = "image/jpeg";

impl HashingService {
    /// Queues `HashBookPages` for the books of a library that enables page hashing and lacks
    /// some of their page hashes.
    pub(crate) async fn find_books_with_missing_page_hash(
        &self,
        library_id: &LibraryId,
        priority: i32,
    ) -> Result<(), HashingError> {
        let mut tx = self.db.begin().await?;
        let Some(library) = LibraryDao::find_by_id(&mut tx, library_id).await? else {
            tx.rollback().await?;
            warn!(library = %library_id, "Cannot find books to hash: Library does not exist");
            return Ok(());
        };
        if !library.hash_pages {
            tx.rollback().await?;
            info!(library = %library_id, "Page hashing is not enabled, skipping");
            return Ok(());
        }
        let books = MediaDao::find_book_ids_with_missing_page_hash(
            &mut tx,
            library_id,
            HASHABLE_MEDIA_TYPES,
            self.page_hashing,
        )
        .await?;
        tx.rollback().await?;
        info!(library = %library_id, count = books.len(), "Found books with missing page hash");
        let tasks: Vec<Task> = books
            .into_iter()
            .map(|book_id| Task::hash_book_pages(book_id).with_priority(priority + 1))
            .collect();
        self.tasks.submit_all(&tasks).await?;
        Ok(())
    }

    /// Hashes the unhashed pages among the first and last `page_hashing` pages of a book.
    ///
    /// As for file hashes, the result is stored only for the file version the database records,
    /// and only when the analysis that listed the pages is still the stored one.
    pub(crate) async fn hash_pages_and_persist(&self, id: &BookId) -> Result<(), HashingError> {
        let mut tx = self.db.begin().await?;
        let Some(book) = BookDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            warn!(book = %id, "Cannot hash pages: Book does not exist");
            return Ok(());
        };
        let library = LibraryDao::find_by_id(&mut tx, &book.library_id)
            .await?
            .expect("A book's library exists (foreign key)");
        let media = MediaDao::find_by_id(&mut tx, id).await?;
        tx.rollback().await?;
        if !library.hash_pages {
            info!(book = %id, "Page hashing is disabled for the library, skipping");
            return Ok(());
        }
        let format = media.as_ref().and_then(readable_format);
        let (Some(mut media), Some(format)) = (media, format) else {
            warn!(book = %id, "Cannot hash pages: Book media is not ready");
            return Ok(());
        };
        let to_hash = pages_to_hash(&media, self.page_hashing);
        if to_hash.is_empty() {
            info!(book = %id, "Book pages are already hashed, skipping");
            return Ok(());
        }
        info!(book = %id, "Hash and persist pages for book");

        let path = book
            .url
            .to_path()
            .ok_or_else(|| HashingError::InvalidUrl(book.url.clone()))?;
        let hashes = if is_recorded(&path, &book).await? {
            let pages: Vec<BookPage> = to_hash
                .iter()
                .map(|&index| media.pages[index].clone())
                .collect();
            let hashes = hash_pages(path.clone(), format, pages).await?;
            is_recorded(&path, &book).await?.then_some(hashes)
        } else {
            None
        };
        let Some(hashes) = hashes else {
            warn!(book = %id, "Book file is not the recorded version, scanning the library");
            self.tasks
                .submit(&Task::scan_library(library.id, true))
                .await?;
            return Ok(());
        };

        let mut tx = self.db.begin_immediate().await?;
        let same_version = BookDao::find_by_id(&mut tx, id)
            .await?
            .is_some_and(|current| {
                (current.file_last_modified, current.file_size)
                    == (book.file_last_modified, book.file_size)
            });
        if !same_version || MediaDao::find_by_id(&mut tx, id).await?.as_ref() != Some(&media) {
            tx.rollback().await?;
            warn!(book = %id, "Book changed during page hashing, dropping the result");
            return Ok(());
        }
        for (index, hash) in to_hash.into_iter().zip(hashes) {
            media.pages[index].file_hash = hash;
        }
        MediaDao::update(&mut tx, id, &media).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// The archive format of a ready book, whose page content Kotlin `getPageContent` reads.
fn readable_format(media: &Media) -> Option<DivinaFormat> {
    if media.status != MediaStatus::Ready {
        return None;
    }
    DivinaFormat::from_media_type(media.media_type.as_deref()?)
}

/// `BookAnalyzer.hashPages`: the indexes of unhashed pages among the first and last
/// `page_hashing` pages.
fn pages_to_hash(media: &Media, page_hashing: i32) -> Vec<usize> {
    let page_hashing = i64::from(page_hashing);
    let page_count = i64::from(media.page_count);
    (0_i64..)
        .zip(&media.pages)
        .filter(|(index, page)| {
            page.file_hash.is_empty()
                && (*index < page_hashing || *index >= page_count - page_hashing)
        })
        .map(|(index, _)| usize::try_from(index).expect("Page indexes fit usize"))
        .collect()
}

async fn hash_pages(
    path: PathBuf,
    format: DivinaFormat,
    pages: Vec<BookPage>,
) -> Result<Vec<String>, HashingError> {
    task::spawn_blocking(move || {
        pages
            .iter()
            .map(|page| hash_page(&path, format, page))
            .collect()
    })
    .await?
}

/// `BookAnalyzer.hashPage`: the file hash of the page's member, JPEGs rewritten first so that
/// their metadata does not count.
fn hash_page(path: &Path, format: DivinaFormat, page: &BookPage) -> Result<String, HashingError> {
    let mut bytes = format.read_entry(path, &page.file_name)?;
    if page.media_type == JPEG {
        bytes = image::rewrite_jpeg(&bytes)?;
    }
    let mut digest = FileDigest::new();
    digest.update(&bytes);
    Ok(digest.finish())
}

#[cfg(test)]
mod tests {
    use komga_types::{BookPage, Media, MediaStatus};

    use super::pages_to_hash;

    fn media(page_count: usize, hashed: &[usize]) -> Media {
        Media {
            status: MediaStatus::Ready,
            media_type: Some("application/zip".to_owned()),
            pages: (0..page_count)
                .map(|index| BookPage {
                    file_name: format!("{index}.jpg"),
                    media_type: "image/jpeg".to_owned(),
                    dimension: None,
                    file_hash: if hashed.contains(&index) {
                        "hashed".to_owned()
                    } else {
                        String::new()
                    },
                    file_size: None,
                })
                .collect(),
            page_count: i32::try_from(page_count).expect("Few pages"),
            files: Vec::new(),
            comment: None,
            epub_divina_compatible: false,
            epub_is_kepub: false,
            extension: None,
        }
    }

    #[test]
    fn hashes_the_unhashed_pages_among_the_first_and_last_ones() {
        assert_eq!(pages_to_hash(&media(30, &[]), 3), [0, 1, 2, 27, 28, 29]);
        assert_eq!(pages_to_hash(&media(30, &[1, 28]), 3), [0, 2, 27, 29]);
        assert_eq!(
            pages_to_hash(&media(4, &[]), 3),
            [0, 1, 2, 3],
            "overlapping ends"
        );
        let all: Vec<usize> = (0..30).collect();
        assert!(pages_to_hash(&media(30, &all), 3).is_empty());
    }
}
