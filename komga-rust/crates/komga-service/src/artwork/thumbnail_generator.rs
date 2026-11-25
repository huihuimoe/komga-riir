//! Kotlin `BookAnalyzer.{generateThumbnail,getPoster}`: the cover as a JPEG, scaled or not.

use std::path::Path;

use komga_archive::divina::{DivinaFormat, EntryError};
use komga_archive::epub::{EpubError, EpubPackage};
use komga_archive::media_type;
use komga_archive::mobi::{self, MOBI_MEDIA_TYPE, MobiError};
use komga_archive::pdf::{PdfDocument, PdfError, RenderedPage};
use komga_image::{self as image, ImageError, ImageType};
use komga_types::{Dimension, Media, ThumbnailSize};

use crate::books::analysis::pdf_extractor;

/// Kotlin `StaticConfiguration.thumbnailType`.
const THUMBNAIL_TYPE: ImageType = ImageType::Jpeg;

pub(super) struct Generated {
    pub bytes: Vec<u8>,
    pub media_type: &'static str,
    /// Kotlin `Dimension(0, 0)` when the header is unreadable.
    pub dimension: Dimension,
}

pub(super) enum GeneratorError {
    NoCover,
    Archive(EntryError),
    Epub(EpubError),
    Mobi(MobiError),
    Pdf(PdfError),
    Image(ImageError),
}

/// The cover of the ready book at `path`, fit within `size`.
pub(super) fn generate(
    path: &Path,
    media: &Media,
    size: ThumbnailSize,
) -> Result<Generated, GeneratorError> {
    let bytes = match cover(path, media)? {
        Cover::Encoded(cover) => image::resize(&cover, THUMBNAIL_TYPE, size.max_edge()),
        Cover::Rendered(page) => image::resize_rgb(
            page.width,
            page.height,
            page.rgb,
            THUMBNAIL_TYPE,
            size.max_edge(),
        ),
    }
    .map_err(GeneratorError::Image)?;
    let dimension = image::read_dimension(bytes.as_slice()).unwrap_or(Dimension {
        width: 0,
        height: 0,
    });
    Ok(Generated {
        bytes,
        media_type: THUMBNAIL_TYPE.media_type(),
        dimension,
    })
}

/// Kotlin `getBookThumbnail` of `OpdsCommonController`: the cover of the ready book at `path`
/// at its own size, as a JPEG.
pub(super) fn poster(path: &Path, media: &Media) -> Result<Vec<u8>, GeneratorError> {
    match cover(path, media)? {
        Cover::Encoded(cover) if is_jpeg(&cover) => Ok(cover),
        Cover::Encoded(cover) => image::convert(&cover, THUMBNAIL_TYPE),
        Cover::Rendered(page) => {
            image::encode_rgb(page.width, page.height, page.rgb, THUMBNAIL_TYPE)
        }
    }
    .map_err(GeneratorError::Image)
}

/// Whether `bytes` is a JPEG already, which is served as is.
pub(super) fn is_jpeg(bytes: &[u8]) -> bool {
    media_type::detect(bytes).is_ok_and(|detected| detected == THUMBNAIL_TYPE.media_type())
}

enum Cover {
    Encoded(Vec<u8>),
    Rendered(RenderedPage),
}

/// Kotlin `BookAnalyzer.getPoster`.
fn cover(path: &Path, media: &Media) -> Result<Cover, GeneratorError> {
    let media_type = media.media_type.as_deref();
    if let Some(format) = media_type.and_then(DivinaFormat::from_media_type) {
        return first_page(format, path, media).map(Cover::Encoded);
    }
    match media_type {
        Some("application/pdf") => {
            let document = PdfDocument::open(path).map_err(GeneratorError::Pdf)?;
            pdf_extractor::render_page(&document, 0)
                .map(Cover::Rendered)
                .map_err(GeneratorError::Pdf)
        }
        Some("application/epub+zip") => {
            let cover = EpubPackage::open(path)
                .and_then(|mut epub| epub.cover())
                .map_err(GeneratorError::Epub)?;
            match cover {
                Some(cover) => Ok(Cover::Encoded(cover)),
                None if media.epub_divina_compatible => {
                    first_page(DivinaFormat::Zip, path, media).map(Cover::Encoded)
                }
                None => Err(GeneratorError::NoCover),
            }
        }
        Some(MOBI_MEDIA_TYPE) => match mobi::read_cover(path).map_err(GeneratorError::Mobi)? {
            Some(cover) => Ok(Cover::Encoded(cover)),
            None => Err(GeneratorError::NoCover),
        },
        _ => Err(GeneratorError::NoCover),
    }
}

/// Kotlin `DivinaExtractor.getPoster`: the first page.
fn first_page(format: DivinaFormat, path: &Path, media: &Media) -> Result<Vec<u8>, GeneratorError> {
    let first = media.pages.first().ok_or(GeneratorError::NoCover)?;
    format
        .read_entry(path, &first.file_name)
        .map_err(GeneratorError::Archive)
}
