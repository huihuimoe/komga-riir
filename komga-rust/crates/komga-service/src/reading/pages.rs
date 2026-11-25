use komga_db::{BookDao, MediaDao};
use komga_image::ImageType;
use komga_types::{Book, BookId, BookPage, KomgaUser, Media, MediaStatus};

use super::page_content::{self, PageContent, PageError, PageOutput, page_error};
use super::{BookSource, ReadingError, ReadingService, pages_as_read};
use crate::support::{access_policy, blocking};

/// Kotlin `BookController.getBookPageThumbnailByNumber`: the longest side, in pixels.
const THUMBNAIL_SIZE: u32 = 300;

impl ReadingService {
    /// The pages of a ready book, in page order; soft-deleted books are readable.
    pub async fn pages(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<Vec<BookPage>, ReadingError> {
        let mut tx = self.db.begin().await?;
        let book = BookDao::find_by_id(&mut tx, id)
            .await?
            .ok_or(ReadingError::NotFound)?;
        if !access_policy::can_access_book(&mut tx, user, &book).await? {
            return Err(ReadingError::Restricted);
        }
        let media = MediaDao::find_by_id(&mut tx, id)
            .await?
            .ok_or_else(|| ReadingError::MissingMedia(id.clone()))?;
        tx.rollback().await?;
        if media.status != MediaStatus::Ready {
            return Err(ReadingError::Unavailable(media.status));
        }
        Ok(pages_as_read(media))
    }

    /// Page `number` (from 1), in the stored format or converted to `convert` (`jpeg` or `png`,
    /// any case; blank means none).
    pub async fn read_page(
        &self,
        user: &KomgaUser,
        source: BookSource,
        number: i32,
        convert: Option<&str>,
    ) -> Result<PageContent, ReadingError> {
        self.check_access(user, &source.book).await?;
        let output = match convert {
            None => PageOutput::Stored,
            Some(format) => match format.to_lowercase().as_str() {
                "" => PageOutput::Stored,
                "jpeg" => PageOutput::Converted(ImageType::Jpeg),
                "png" => PageOutput::Converted(ImageType::Png),
                _ => return Err(ReadingError::InvalidConversion(format.to_owned())),
            },
        };
        self.serve(source, number, output).await
    }

    /// Page `number` (from 1) scaled down to a JPEG thumbnail.
    pub async fn read_page_thumbnail(
        &self,
        user: &KomgaUser,
        source: BookSource,
        number: i32,
    ) -> Result<PageContent, ReadingError> {
        self.check_access(user, &source.book).await?;
        self.serve(source, number, PageOutput::Resized(THUMBNAIL_SIZE))
            .await
    }

    /// Kotlin `BookLifecycle.getBookPage` as `PageHashLifecycle.getPage` calls it: page
    /// `number` (from 1) of any book, in the stored format or resized to a JPEG of at most
    /// `resize` pixels on its longest side.
    pub(crate) async fn read_book_page(
        &self,
        id: &BookId,
        number: i32,
        resize: Option<u32>,
    ) -> Result<PageContent, ReadingError> {
        let source = self.book_source(id).await?;
        let output = resize.map_or(PageOutput::Stored, PageOutput::Resized);
        self.serve(source, number, output).await
    }

    /// The digits of the first EAN-13 barcode on page `number` (from 1) of `id`.
    pub(crate) async fn read_book_page_ean13(
        &self,
        id: &BookId,
        number: i32,
    ) -> Result<Option<String>, ReadingError> {
        let source = self.book_source(id).await?;
        let location = page_content::page_to_read(&source.media, number)?;
        let from = location.media_type().to_owned();
        let book = source.book;
        let path = book
            .url
            .to_path()
            .ok_or_else(|| ReadingError::InvalidUrl(book.url.clone()))?;
        let pdf_cache = self.pdf_cache.clone();
        let book_id = book.id.clone();
        blocking::run(move || page_content::read_ean13(&pdf_cache, &book_id, &path, &location))
            .await?
            .map_err(|error| page_error(&book, number, from, error))
    }

    async fn serve(
        &self,
        source: BookSource,
        number: i32,
        output: PageOutput,
    ) -> Result<PageContent, ReadingError> {
        let location = page_content::page_to_read(&source.media, number)?;
        let from = location.media_type().to_owned();
        let book = source.book;
        let path = book
            .url
            .to_path()
            .ok_or_else(|| ReadingError::InvalidUrl(book.url.clone()))?;
        let pdf_cache = self.pdf_cache.clone();
        let id = book.id.clone();
        blocking::run(move || page_content::read(&pdf_cache, &id, &path, &location, output))
            .await?
            .map_err(|error| page_error(&book, number, from, error))
    }

    /// Page `number` (from 1) of a book that is in no library, as stored.
    pub(crate) async fn read_transient_page(
        &self,
        book: Book,
        media: Media,
        number: i32,
    ) -> Result<PageContent, ReadingError> {
        let source = BookSource {
            book,
            media,
            last_modified: 0,
        };
        self.serve(source, number, PageOutput::Stored).await
    }

    /// Page `number` (from 1) as a single-page document, which only PDF books have.
    pub async fn read_raw_page(
        &self,
        user: &KomgaUser,
        source: BookSource,
        number: i32,
    ) -> Result<PageContent, ReadingError> {
        self.check_access(user, &source.book).await?;
        if !page_content::is_pdf(&source.media) {
            return Err(ReadingError::RawUnsupported);
        }
        let index = page_content::page_index(&source.media, number)?;
        let book = source.book;
        let path = book
            .url
            .to_path()
            .ok_or_else(|| ReadingError::InvalidUrl(book.url.clone()))?;
        let pdf_cache = self.pdf_cache.clone();
        let id = book.id.clone();
        blocking::run(move || page_content::read_pdf_page(&pdf_cache, &id, &path, index))
            .await?
            .map_err(|error| page_error(&book, number, String::new(), PageError::Pdf(error)))
    }
}
