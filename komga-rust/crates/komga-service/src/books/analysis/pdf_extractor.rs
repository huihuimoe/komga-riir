//! Kotlin `PdfExtractor` with `StaticConfiguration`'s `pdfImageType` and `pdfResolution`.

use std::path::Path;

use komga_archive::pdf::{PageSize, PdfDocument, PdfError, RenderedPage};
use komga_image::{self as image, ImageType};
use komga_types::{BookPage, Dimension};

/// The format PDF pages are rendered to.
pub(crate) const IMAGE_TYPE: ImageType = ImageType::Jpeg;
/// The shorter side of a rendered page, in pixels.
const RESOLUTION: f32 = 3200.0;

/// `getPages`: one page per PDF page, named by its number from 1, with its size in points when
/// `analyze_dimensions` is set.
///
/// # Errors
/// The file cannot be read or parsed.
pub(super) fn pages(path: &Path, analyze_dimensions: bool) -> Result<Vec<BookPage>, PdfError> {
    let document = PdfDocument::open(path)?;
    let pages = document
        .page_sizes()
        .into_iter()
        .enumerate()
        .map(|(index, PageSize { width, height })| BookPage {
            file_name: (index + 1).to_string(),
            media_type: String::new(),
            dimension: analyze_dimensions.then(|| Dimension {
                width: round(width),
                height: round(height),
            }),
            file_hash: String::new(),
            file_size: None,
        })
        .collect();
    Ok(pages)
}

/// Page `index` (from 0) rendered to RGB pixels at [`RESOLUTION`].
pub(crate) fn render_page(document: &PdfDocument, index: usize) -> Result<RenderedPage, PdfError> {
    document.render(index, RESOLUTION, image::MAX_PIXELS)
}

/// Kotlin `BookAnalyzer.getPdfPagesDynamic`: the stored pages as rendered.
pub(crate) fn rendered_pages(pages: Vec<BookPage>) -> Vec<BookPage> {
    pages
        .into_iter()
        .map(|page| BookPage {
            media_type: IMAGE_TYPE.media_type().to_owned(),
            dimension: page.dimension.map(scale_dimension),
            ..page
        })
        .collect()
}

/// `scaleDimension`.
fn scale_dimension(dimension: Dimension) -> Dimension {
    let (width, height) = (f64::from(dimension.width), f64::from(dimension.height));
    let scale = f64::from(RESOLUTION) / width.min(height);
    Dimension {
        width: round(width * scale),
        height: round(height * scale),
    }
}

/// Rounds a non-negative page size to `u32`, saturating out-of-range values.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "saturating for non-negative page dimensions"
)]
fn round(value: f64) -> u32 {
    value.round() as u32
}

#[cfg(test)]
mod tests {
    use komga_types::{BookPage, Dimension};

    use super::rendered_pages;

    fn page(dimension: Option<(u32, u32)>) -> BookPage {
        BookPage {
            file_name: "1".to_owned(),
            media_type: String::new(),
            dimension: dimension.map(|(width, height)| Dimension { width, height }),
            file_hash: String::new(),
            file_size: None,
        }
    }

    /// Expected sizes from Kotlin `PdfExtractor.scaleDimension` with a resolution of 3200.
    #[test]
    fn rendered_pages_are_jpeg_scaled_to_3200_on_the_shorter_side() {
        let sizes = [(595, 842), (300, 201), (512, 512)];
        let pages = rendered_pages(sizes.map(|size| page(Some(size))).into());
        let rendered: Vec<_> = pages
            .iter()
            .map(|page| {
                assert_eq!(page.media_type, "image/jpeg");
                page.dimension.map(|size| (size.width, size.height))
            })
            .collect();
        assert_eq!(
            rendered,
            [Some((3200, 4528)), Some((4776, 3200)), Some((3200, 3200))]
        );
        assert_eq!(rendered_pages(vec![page(None)])[0].dimension, None);
    }
}
