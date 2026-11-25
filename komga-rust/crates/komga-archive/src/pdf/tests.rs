//! Page counts and sizes as PDFBox 3.0.8 reads the fixtures: `komga.pdf` is Komga's own test
//! document; `pages.pdf` has a plain page, a cropped page, a page rotated by 90 degrees and a crop
//! box reaching beyond the media box; `user-password.pdf` and `owner-password.pdf` are
//! `komga.pdf` encrypted (RC4, revision 3) with and without a user password; `truncated.pdf` is
//! its first 600 bytes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{MAX_FILE_BYTES, PageSize, PdfDocument, PdfError};

/// Kotlin `pdfResolution`.
const RESOLUTION: f32 = 3200.0;
const MAX_PIXELS: u64 = 128 * 1024 * 1024;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pdf")
        .join(name)
}

/// Sizes rounded as Kotlin `Float.roundToInt`, as `PdfExtractor.getPages` stores them.
fn rounded_sizes(name: &str) -> Vec<(i64, i64)> {
    let document = PdfDocument::open(&fixture(name)).expect("Readable fixture");
    document
        .page_sizes()
        .iter()
        .map(|&PageSize { width, height }| (width.round() as i64, height.round() as i64))
        .collect()
}

#[test]
fn page_sizes_are_the_crop_box_within_the_media_box_before_rotation() {
    assert_eq!(rounded_sizes("komga.pdf"), [(512, 512)]);
    assert_eq!(
        rounded_sizes("pages.pdf"),
        [(595, 842), (300, 201), (400, 300), (100, 150)]
    );
}

#[test]
fn owner_password_only_and_truncated_files_are_read_as_pdfbox_reads_them() {
    assert_eq!(rounded_sizes("owner-password.pdf"), [(512, 512)]);
    assert_eq!(rounded_sizes("truncated.pdf"), [(512, 512)]);
}

#[test]
fn user_password_files_are_encrypted() {
    let error = PdfDocument::open(&fixture("user-password.pdf")).err();
    assert!(matches!(error, Some(PdfError::Encrypted)), "{error:?}");
}

#[test]
fn files_that_are_not_pdf_are_unreadable() {
    let error = PdfDocument::open(&fixture("../rar/rar4.rar")).err();
    assert!(matches!(error, Some(PdfError::Unreadable(_))), "{error:?}");
}

#[test]
fn missing_files_stay_io_errors() {
    let error = PdfDocument::open(&fixture("missing.pdf")).err();
    assert!(
        matches!(&error, Some(PdfError::Io(cause)) if cause.kind() == std::io::ErrorKind::NotFound),
        "{error:?}"
    );
}

#[test]
fn files_over_the_size_limit_are_not_read() {
    let file = tempfile::NamedTempFile::new().expect("Temporary file");
    // Sparse, so the test writes nothing.
    file.as_file()
        .set_len(MAX_FILE_BYTES + 1)
        .expect("Sparse file");
    let error = PdfDocument::open(file.path()).err();
    assert!(
        matches!(error, Some(PdfError::TooLarge(size)) if size == MAX_FILE_BYTES + 1),
        "{error:?}"
    );
}

/// Sizes and white background from PDFBox 3.0.8 `PDFRenderer.renderImage(index, scale, RGB)`.
#[test]
fn pages_render_rotated_on_white_with_floored_sides_as_pdfbox_renders_them() {
    let document = PdfDocument::open(&fixture("pages.pdf")).expect("Readable fixture");
    let sizes: Vec<_> = (0..4)
        .map(|index| {
            let page = document
                .render(index, RESOLUTION, MAX_PIXELS)
                .expect("Renderable page");
            assert_eq!(
                page.rgb.len(),
                page.width as usize * page.height as usize * 3
            );
            assert!(
                page.rgb.iter().all(|&channel| channel == 255),
                "Blank page {index}"
            );
            (page.width, page.height)
        })
        .collect();
    assert_eq!(
        sizes,
        [(3200, 4525), (4791, 3200), (3200, 4266), (3200, 4800)]
    );
}

#[test]
fn page_content_is_drawn() {
    let document = PdfDocument::open(&fixture("komga.pdf")).expect("Readable fixture");
    let page = document
        .render(0, RESOLUTION, MAX_PIXELS)
        .expect("Renderable page");
    assert_eq!((page.width, page.height), (3200, 3200));
    assert!(
        page.rgb.iter().any(|&channel| channel < 128),
        "Nothing drawn"
    );
}

#[test]
fn missing_pages_are_out_of_bounds() {
    let document = PdfDocument::open(&fixture("komga.pdf")).expect("Readable fixture");
    let error = document.render(1, RESOLUTION, MAX_PIXELS).err();
    assert!(
        matches!(error, Some(PdfError::PageOutOfBounds(1))),
        "{error:?}"
    );
}

#[test]
fn pages_over_the_pixel_limit_are_not_rendered() {
    let document = PdfDocument::open(&fixture("komga.pdf")).expect("Readable fixture");
    let error = document.render(0, RESOLUTION, 3200 * 3200 - 1).err();
    assert!(
        matches!(
            error,
            Some(PdfError::PageTooLarge {
                width: 3200.0,
                height: 3200.0
            })
        ),
        "{error:?}"
    );
}

/// hayro's canvas sides are `u16`: a 1:30 page would be 96000 pixels high at 3200 wide.
#[test]
fn pages_over_the_canvas_side_limit_are_not_clipped() {
    let mut file = tempfile::NamedTempFile::new().expect("Temporary file");
    file.write_all(
        b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
          2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
          3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 10 300] >> endobj\n\
          trailer << /Root 1 0 R >>\n%%EOF\n",
    )
    .expect("Written PDF");
    let document = PdfDocument::open(file.path()).expect("Repaired PDF");
    let error = document.render(0, RESOLUTION, u64::MAX).err();
    assert!(
        matches!(error, Some(PdfError::PageTooLarge { width: 3200.0, height }) if height > 65535.0),
        "{error:?}"
    );
}

/// Each page alone keeps its boxes and rotation, and is drawn as in the whole document.
#[test]
fn extracted_pages_reparse_as_the_same_single_page() {
    for name in ["pages.pdf", "komga.pdf", "owner-password.pdf"] {
        let document = PdfDocument::open(&fixture(name)).expect("Readable fixture");
        for (index, size) in document.page_sizes().into_iter().enumerate() {
            let bytes = document.page_pdf(index).expect("Existing page");
            let page = PdfDocument::from_bytes(Arc::new(bytes)).expect("Readable extracted page");
            assert_eq!(page.page_sizes(), [size], "{name} page {index}");
            let original = document
                .render(index, 400.0, MAX_PIXELS)
                .expect("Renderable");
            let extracted = page.render(0, 400.0, MAX_PIXELS).expect("Renderable");
            assert_eq!(
                (extracted.width, extracted.height),
                (original.width, original.height),
                "{name} page {index}"
            );
            assert!(extracted.rgb == original.rgb, "{name} page {index} differs");
        }
    }
}

#[test]
fn missing_pages_are_not_extracted() {
    let document = PdfDocument::open(&fixture("komga.pdf")).expect("Readable fixture");
    let error = document.page_pdf(1).err();
    assert!(
        matches!(error, Some(PdfError::PageOutOfBounds(1))),
        "{error:?}"
    );
}
