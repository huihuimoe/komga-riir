//! PDF documents as Komga's PDFBox 3.0.8 `PdfExtractor` reads them, parsed by `hayro`.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro::vello_cpu::Pixels;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::peniko::ImageAlphaType;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use hayro_write::pdf_writer::{self, Ref};
use hayro_write::{ChunkSettings, ExtractionQuery};

/// Largest file read.
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;

/// Errors from opening a document.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("PDF file of {0} bytes is larger than {MAX_FILE_BYTES} bytes")]
    TooLarge(u64),
    /// A user password is needed; PDFBox, like hayro, opens owner-password-only files.
    #[error("Password-protected PDF")]
    Encrypted,
    #[error("Unreadable PDF: {0:?}")]
    Unreadable(LoadPdfError),
    /// PDFBox `getPage` throws `IndexOutOfBoundsException`.
    #[error("Page index {0} does not exist")]
    PageOutOfBounds(usize),
    /// The rendered page exceeds the pixel limit or hayro's 65535-pixel canvas sides.
    #[error("Rendered page of {width}x{height} pixels is too large")]
    PageTooLarge { width: f32, height: f32 },
}

/// PDFBox `PDPage.getCropBox`: the crop box clipped to the media box, in points, before the
/// page rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    pub width: f64,
    pub height: f64,
}

/// A page rendered on white, as 8-bit RGB rows.
pub struct RenderedPage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// The whole of `file`, which hayro needs in memory where PDFBox reads on demand.
///
/// # Errors
/// The file cannot be read or is larger than [`MAX_FILE_BYTES`].
pub fn read_file(file: File) -> Result<Vec<u8>, PdfError> {
    let size = file.metadata()?.len();
    if size > MAX_FILE_BYTES {
        return Err(PdfError::TooLarge(size));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    // The file may grow after its size was read.
    let read = file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if read as u64 > MAX_FILE_BYTES {
        return Err(PdfError::TooLarge(read as u64));
    }
    Ok(bytes)
}

pub struct PdfDocument {
    pdf: Pdf,
}

impl PdfDocument {
    /// Reads and parses the file at `path`.
    ///
    /// # Errors
    /// See [`read_file`] and [`PdfDocument::from_bytes`].
    pub fn open(path: &Path) -> Result<Self, PdfError> {
        Self::from_bytes(Arc::new(read_file(File::open(path)?)?))
    }

    /// Parses a whole file, which PDFBox repairs when its cross-reference data is damaged, as
    /// hayro does.
    ///
    /// # Errors
    /// The file needs a password or is not a readable PDF.
    pub fn from_bytes(bytes: Arc<Vec<u8>>) -> Result<Self, PdfError> {
        let pdf = Pdf::new(bytes).map_err(|error| match error {
            LoadPdfError::Decryption(DecryptionError::PasswordProtected) => PdfError::Encrypted,
            error => PdfError::Unreadable(error),
        })?;
        Ok(Self { pdf })
    }

    /// The size of each page, in page order.
    #[must_use]
    pub fn page_sizes(&self) -> Vec<PageSize> {
        self.pdf
            .pages()
            .iter()
            .map(|page| {
                let crop_box = page.intersected_crop_box();
                PageSize {
                    width: crop_box.width(),
                    height: crop_box.height(),
                }
            })
            .collect()
    }

    /// PDFBox `PDFRenderer.renderImage` in RGB: page `index` (from 0), rotated, scaled so that
    /// the shorter side of its crop box is `resolution` pixels. Sides are floored, as in PDFBox.
    ///
    /// # Errors
    /// The page does not exist, or its canvas would exceed `max_pixels` or hayro's side limit.
    pub fn render(
        &self,
        index: usize,
        resolution: f32,
        max_pixels: u64,
    ) -> Result<RenderedPage, PdfError> {
        let page = self
            .pdf
            .pages()
            .get(index)
            .ok_or(PdfError::PageOutOfBounds(index))?;
        let (crop_width, crop_height) = page.base_dimensions();
        let scale = resolution / crop_width.min(crop_height);
        let (width, height) = page.render_dimensions();
        let (width, height) = ((width * scale).floor(), (height * scale).floor());
        // hayro casts the sides to `u16`, which would silently clip larger pages.
        let max_side = f32::from(u16::MAX);
        if !(0.0..=max_side).contains(&width) || !(0.0..=max_side).contains(&height) {
            return Err(PdfError::PageTooLarge { width, height });
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Both sides are finite non-negative whole numbers within `u16`"
        )]
        let pixels = (width as u64) * (height as u64);
        if pixels > max_pixels {
            return Err(PdfError::PageTooLarge { width, height });
        }
        let mut pixmap = hayro::render(
            page,
            &RenderCache::new(),
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        let width = pixmap.width().into();
        let height = pixmap.height().into();
        // Every pixel is opaque on the white background, so premultiplied channels are final.
        pixmap.set_may_have_transparency(false);
        let Pixels::Rgb8(rgb) = pixmap.try_take_rgb8(ImageAlphaType::AlphaPremultiplied) else {
            unreachable!("The opaque pixmap converts to RGB8");
        };
        Ok(RenderedPage { width, height, rgb })
    }

    /// PDFBox `PageExtractor` of page `index` (from 0) alone, saved as a new document. Its
    /// boxes, rotation, contents, resources and transparency group are kept, as hayro-write
    /// copies them; annotations and document-level data are not.
    ///
    /// # Errors
    /// The page does not exist.
    pub fn page_pdf(&self, index: usize) -> Result<Vec<u8>, PdfError> {
        let mut next_ref = Ref::new(1);
        let catalog = next_ref.bump();
        let extracted = hayro_write::extract(
            &self.pdf,
            Box::new(|| next_ref.bump()),
            ChunkSettings::default(),
            &[ExtractionQuery::new_page(index)],
        )
        .map_err(|_| PdfError::PageOutOfBounds(index))?;
        let page = extracted.root_refs[0].expect("`extract` fails on a missing page");
        let mut pdf = pdf_writer::Pdf::new();
        pdf.extend(&extracted.chunk);
        pdf.catalog(catalog).pages(extracted.page_tree_parent_ref);
        pdf.pages(extracted.page_tree_parent_ref)
            .kids([page])
            .count(1);
        Ok(pdf.finish())
    }
}

#[cfg(test)]
mod tests;
