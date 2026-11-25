//! Kotlin `BookAnalyzer.getPageContent` and the format choice of `BookLifecycle.getBookPage`.

use std::io;
use std::path::Path;

use komga_archive::divina::{DivinaFormat, EntryError};
use komga_archive::media_type;
use komga_archive::mobi::MOBI_MEDIA_TYPE;
use komga_archive::pdf::{PdfError, RenderedPage};
use komga_image::{self as image, ImageError, ImageType};
use komga_types::{Book, BookId, BookPage, Media, MediaProfile, MediaStatus};
use tracing::{error, warn};

use super::ReadingError;
use super::pdf_cache::PdfCache;
use crate::books::analysis::pdf_extractor;

const PDF: &str = "application/pdf";
const EPUB: &str = "application/epub+zip";

/// A page's bytes and media type.
pub struct PageContent {
    pub bytes: Vec<u8>,
    pub media_type: String,
    /// Kotlin `ContentDetector.mediaTypeToExtension` of `media_type`, with its dot.
    pub extension: Option<&'static str>,
}

impl PageContent {
    fn new(bytes: Vec<u8>, media_type: String) -> Self {
        Self {
            bytes,
            extension: media_type::extension(&media_type),
            media_type,
        }
    }
}

/// Kotlin `Media.profile == MediaProfile.PDF`.
pub(super) fn is_pdf(media: &Media) -> bool {
    media.media_type.as_deref() == Some(PDF)
}

/// Kotlin `Media.profile == MediaProfile.EPUB` (includes MOBI per spec §16.4).
pub(super) fn is_epub(media: &Media) -> bool {
    MediaProfile::from_media_type(media.media_type.as_deref()) == Some(MediaProfile::Epub)
}

/// Where a page's bytes come from.
pub(super) enum PageLocation {
    Entry {
        format: DivinaFormat,
        page: BookPage,
    },
    /// A PDF page by index from 0, rendered when read.
    Pdf(usize),
}

impl PageLocation {
    /// `BookLifecycle.getBookPage`'s `pageMediaType`.
    pub(super) fn media_type(&self) -> &str {
        match self {
            Self::Entry { page, .. } => &page.media_type,
            Self::Pdf(_) => pdf_extractor::IMAGE_TYPE.media_type(),
        }
    }
}

/// The checks `getPageContent` and `getPageContentRaw` make first, in their order: a ready
/// media and a number within the page count. Returns the page index from 0.
pub(super) fn page_index(media: &Media, number: i32) -> Result<usize, ReadingError> {
    if media.status != MediaStatus::Ready {
        return Err(ReadingError::MediaNotReady);
    }
    if number > media.page_count || number <= 0 {
        return Err(ReadingError::PageOutOfBounds);
    }
    Ok(usize::try_from(number - 1).expect("The page number is positive"))
}

/// [`page_index`], then the checks `getPageContent` makes before reading: a known format and a
/// stored page.
pub(super) fn page_to_read(media: &Media, number: i32) -> Result<PageLocation, ReadingError> {
    let index = page_index(media, number)?;
    let media_type = media.media_type.as_deref();
    let format = match media_type {
        Some(PDF) => return Ok(PageLocation::Pdf(index)),
        // `EpubExtractor.getEntryStream` reads members as `ZipExtractor` does.
        Some(EPUB) if media.epub_divina_compatible => DivinaFormat::Zip,
        Some(EPUB | MOBI_MEDIA_TYPE) => return Err(ReadingError::EpubPagesUnsupported),
        _ => media_type
            .and_then(DivinaFormat::from_media_type)
            .ok_or(ReadingError::MediaNotReady)?,
    };
    media
        .pages
        .get(index)
        .map(|page| PageLocation::Entry {
            format,
            page: page.clone(),
        })
        .ok_or(ReadingError::PageOutOfBounds)
}

/// What a page is served as.
pub(super) enum PageOutput {
    Stored,
    /// Converted to the format, unless already in it.
    Converted(ImageType),
    /// A JPEG of at most this many pixels on its longest side.
    Resized(u32),
}

/// Why a page could not be served.
pub(super) enum PageError {
    Archive(EntryError),
    Pdf(PdfError),
    /// Kotlin `ImageConversionException`: the page's format cannot be decoded into the target.
    UnreadableFormat(ImageType),
    Conversion(ImageError),
}

/// Logs and classifies a failed page read; `from` is the page's media type.
pub(super) fn page_error(book: &Book, number: i32, from: String, error: PageError) -> ReadingError {
    match error {
        PageError::Archive(error) if error.is_file_not_found() => {
            warn!(book = %book.id, url = %book.url, "File not found");
            ReadingError::FileNotFound
        }
        PageError::Archive(error) => {
            warn!(book = %book.id, url = %book.url, %error, "Cannot read page");
            ReadingError::Archive(error)
        }
        PageError::Pdf(PdfError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            warn!(book = %book.id, url = %book.url, "File not found");
            ReadingError::FileNotFound
        }
        // The file lost pages since its analysis.
        PageError::Pdf(PdfError::PageOutOfBounds(_)) => ReadingError::PageOutOfBounds,
        PageError::Pdf(error) => {
            error!(book = %book.id, url = %book.url, number, %error, "Cannot read PDF page");
            ReadingError::Pdf(error)
        }
        PageError::UnreadableFormat(target) => ReadingError::UnreadableFormat {
            number,
            book: book.id.clone(),
            from,
            to: target.media_type(),
        },
        PageError::Conversion(error) => {
            error!(book = %book.id, number, %from, %error, "Page conversion failed");
            ReadingError::Conversion(error)
        }
    }
}

/// Reads the page at `location` in `book` at `path` as `output`. Blocking.
pub(super) fn read(
    pdf_cache: &PdfCache,
    book: &BookId,
    path: &Path,
    location: &PageLocation,
    output: PageOutput,
) -> Result<PageContent, PageError> {
    let from = location.media_type();
    let (bytes, media_type) = match location {
        PageLocation::Entry { format, page } => {
            let bytes = format
                .read_entry(path, &page.file_name)
                .map_err(PageError::Archive)?;
            match output {
                PageOutput::Stored => (bytes, from),
                PageOutput::Converted(target) if !image::can_read(from) => {
                    return Err(PageError::UnreadableFormat(target));
                }
                PageOutput::Converted(target) if target.media_type() == from => (bytes, from),
                PageOutput::Converted(target) => (
                    image::convert(&bytes, target).map_err(PageError::Conversion)?,
                    target.media_type(),
                ),
                PageOutput::Resized(size) => (
                    image::resize(&bytes, ImageType::Jpeg, size).map_err(PageError::Conversion)?,
                    ImageType::Jpeg.media_type(),
                ),
            }
        }
        PageLocation::Pdf(index) => {
            let document = pdf_cache.open(book, path).map_err(PageError::Pdf)?;
            let rendered = pdf_extractor::render_page(&document, *index).map_err(PageError::Pdf)?;
            let (target, size) = match output {
                PageOutput::Stored => (pdf_extractor::IMAGE_TYPE, None),
                PageOutput::Converted(target) => (target, None),
                PageOutput::Resized(size) => (ImageType::Jpeg, Some(size)),
            };
            let RenderedPage { width, height, rgb } = rendered;
            let bytes = match size {
                Some(size) => image::resize_rgb(width, height, rgb, target, size),
                None => image::encode_rgb(width, height, rgb, target),
            }
            .map_err(PageError::Conversion)?;
            (bytes, target.media_type())
        }
    };
    Ok(PageContent::new(bytes, media_type.to_owned()))
}

/// Reads the first EAN-13 barcode on the page at `location` in `book` at `path`. Blocking.
pub(super) fn read_ean13(
    pdf_cache: &PdfCache,
    book: &BookId,
    path: &Path,
    location: &PageLocation,
) -> Result<Option<String>, PageError> {
    match location {
        PageLocation::Entry { format, page } => {
            let bytes = format
                .read_entry(path, &page.file_name)
                .map_err(PageError::Archive)?;
            image::read_ean13(&bytes).map_err(PageError::Conversion)
        }
        PageLocation::Pdf(index) => {
            let document = pdf_cache.open(book, path).map_err(PageError::Pdf)?;
            let rendered = pdf_extractor::render_page(&document, *index).map_err(PageError::Pdf)?;
            Ok(image::read_ean13_rgb(
                rendered.width,
                rendered.height,
                &rendered.rgb,
            ))
        }
    }
}

/// Kotlin `PdfExtractor.getPageContentAsPdf`: page `index` (from 0) of the PDF `book` at `path`
/// as a document of its own. Blocking.
pub(super) fn read_pdf_page(
    pdf_cache: &PdfCache,
    book: &BookId,
    path: &Path,
    index: usize,
) -> Result<PageContent, PdfError> {
    let bytes = pdf_cache.open(book, path)?.page_pdf(index)?;
    Ok(PageContent::new(bytes, PDF.to_owned()))
}
