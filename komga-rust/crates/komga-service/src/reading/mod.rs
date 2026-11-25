//! Reading book content: Kotlin `BookController.getBookPages` and the page reads of
//! `CommonBookController`, `BookLifecycle.getBookPage`, and `BookAnalyzer`, and EPUB resources.

mod book_file;
mod epub_resource;
mod error;
mod file_cache;
pub(crate) mod fonts;
mod mobi_cache;
mod page_content;
mod page_types;
mod pages;
mod pdf_cache;
mod positions;
pub(crate) mod progress;
mod series_archive;
mod webpub;

use jiff::tz::TimeZone;
use komga_db::{BookDao, Database, MediaDao};
use komga_types::{Book, BookId, BookPage, KomgaUser, Media, UserRole};

pub use book_file::BookFile;
pub use epub_resource::{EpubResource, is_font as is_epub_font};
pub use error::ReadingError;
pub use page_content::PageContent;
pub use webpub::{ManifestSource, can_convert_to_jpeg};

use crate::books::analysis::pdf_extractor;
use crate::support::access_policy;
use mobi_cache::MobiCache;
use pdf_cache::{IDLE_EXPIRY, PdfCache};

/// A book and its media as read before authorization, so that conditional requests are answered
/// first, as in Kotlin.
pub struct BookSource {
    book: Book,
    media: Media,
    last_modified: i64,
}

impl BookSource {
    /// Kotlin `getBookLastModified`: the media's modification time in epoch milliseconds.
    #[must_use]
    pub fn last_modified(&self) -> i64 {
        self.last_modified
    }

    #[must_use]
    pub fn book_name(&self) -> &str {
        &self.book.name
    }

    /// Kotlin `Media.profile == MediaProfile.PDF`.
    #[must_use]
    pub fn is_pdf(&self) -> bool {
        page_content::is_pdf(&self.media)
    }
}

/// The pages of `media`, PDF pages as rendered.
pub(crate) fn pages_as_read(media: Media) -> Vec<BookPage> {
    if page_content::is_pdf(&media) {
        pdf_extractor::rendered_pages(media.pages)
    } else {
        media.pages
    }
}

#[derive(Clone)]
pub struct ReadingService {
    db: Database,
    pdf_cache: PdfCache,
    mobi_cache: MobiCache,
}

impl ReadingService {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self {
            db,
            pdf_cache: PdfCache::new(),
            mobi_cache: MobiCache::new(),
        }
    }

    /// Drops cached bytes of PDF and MOBI files unread lately; the caller runs it periodically.
    pub fn evict_idle_files(&self) {
        self.pdf_cache.evict_idle(IDLE_EXPIRY);
        self.mobi_cache.evict_idle(IDLE_EXPIRY);
    }

    /// The book whose page a user with `PAGE_STREAMING` asks for.
    pub async fn page_source(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<BookSource, ReadingError> {
        if !user.has_role(UserRole::PageStreaming) {
            return Err(ReadingError::Forbidden);
        }
        self.book_source(id).await
    }

    /// The book whose page thumbnail or EPUB resource is asked for, which needs no role.
    pub async fn book_source(&self, id: &BookId) -> Result<BookSource, ReadingError> {
        let mut tx = self.db.begin().await?;
        let book = BookDao::find_by_id(&mut tx, id)
            .await?
            .ok_or(ReadingError::NotFound)?;
        let media = MediaDao::find_by_id(&mut tx, id).await?;
        let last_modified = MediaDao::find_last_modified_date(&mut tx, id).await?;
        tx.rollback().await?;
        let (Some(media), Some(last_modified)) = (media, last_modified) else {
            return Err(ReadingError::MissingMedia(id.clone()));
        };
        let last_modified = last_modified
            .to_zoned_datetime(TimeZone::UTC)
            .expect("Stored dates are in range")
            .timestamp()
            .as_millisecond();
        Ok(BookSource {
            book,
            media,
            last_modified,
        })
    }

    async fn check_access(&self, user: &KomgaUser, book: &Book) -> Result<(), ReadingError> {
        let mut tx = self.db.begin().await?;
        let allowed = access_policy::can_access_book(&mut tx, user, book).await?;
        tx.rollback().await?;
        if allowed {
            Ok(())
        } else {
            Err(ReadingError::Restricted)
        }
    }
}
