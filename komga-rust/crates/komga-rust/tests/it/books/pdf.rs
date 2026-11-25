//! PDF books (`PdfExtractor`) from scan to analysis, page list, rendered pages and cover.

use std::fs::{self, File};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use image::{GenericImageView, ImageFormat};
use komga_archive::pdf::{PageSize, PdfDocument};
use reqwest::header::{ACCEPT, CONTENT_DISPOSITION, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde_json::json;

use super::library_with_books;
use super::reading::{Fetched, fetch};
use super::thumbnails::assert_jpeg;
use crate::support::TestServer;
use crate::support::cbz;
use crate::support::db::content::{book, media, pages};
use crate::support::openapi;
use crate::support::tasks::wait_for_generations;

const PDF: &str = "application/pdf";

fn pdf(name: &str) -> Vec<u8> {
    cbz::fixture(&format!("komga-archive/tests/fixtures/pdf/{name}"))
}

#[tokio::test]
async fn pdf_books_are_analyzed_and_listed_as_rendered_pages() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/pages.pdf", pdf("pages.pdf")),
            ("Series/owner.pdf", pdf("owner-password.pdf")),
            ("Series/truncated.pdf", pdf("truncated.pdf")),
            ("Series/user.pdf", pdf("user-password.pdf")),
            ("Series/not-a-pdf.pdf", b"%PDF-1.4\n%%EOF\n".to_vec()),
        ],
    )
    .await;
    let row = |status: &str, comment: Option<&str>, page_count| {
        (
            status.to_owned(),
            Some(PDF.to_owned()),
            comment.map(str::to_owned),
            page_count,
        )
    };
    for (name, expected) in [
        ("pages", row("READY", None, 4)),
        // PDFBox reads both: owner-password-only files decrypt, damaged ones are repaired.
        ("owner", row("READY", None, 1)),
        ("truncated", row("READY", None, 1)),
        ("user", row("ERROR", Some("ERR_1005"), 0)),
        ("not-a-pdf", row("ERROR", Some("ERR_1005"), 0)),
    ] {
        let (book_id, _) = book(&pool, name).await;
        assert_eq!(media(&pool, &book_id).await, expected, "{name}");
    }

    // Stored as `getPages` returns them: untyped, sized in points before rotation.
    let (book_id, _) = book(&pool, "pages").await;
    let stored = |name: &str, width, height| {
        (
            name.to_owned(),
            String::new(),
            Some(width),
            Some(height),
            String::new(),
            None,
        )
    };
    assert_eq!(
        pages(&pool, &book_id).await,
        [
            stored("1", 595, 842),
            stored("2", 300, 201),
            stored("3", 400, 300),
            stored("4", 100, 150),
        ]
    );

    // Listed as `getPdfPagesDynamic` renders them.
    let response = server
        .admin(Method::GET, &format!("/api/v1/books/{book_id}/pages"))
        .send()
        .await
        .expect("List");
    assert_eq!(response.status(), StatusCode::OK);
    let page = |number: u32, width: u32, height: u32| {
        json!({"number": number, "fileName": number.to_string(), "mediaType": "image/jpeg",
               "width": width, "height": height, "sizeBytes": null, "size": ""})
    };
    assert_eq!(
        openapi::checked(Method::GET, response).await,
        json!([
            page(1, 3200, 4528),
            page(2, 4776, 3200),
            page(3, 4267, 3200),
            page(4, 3200, 4800),
        ])
    );
    server.stop().await;
}

/// The page as an image: reqwest's default `*/*` would negotiate the raw PDF page.
async fn page(server: &TestServer, book_id: &str, path: &str) -> Fetched {
    page_accepting(server, book_id, path, "image/*").await
}

async fn page_accepting(server: &TestServer, book_id: &str, path: &str, accept: &str) -> Fetched {
    let path = format!("/api/v1/books/{book_id}/pages/{path}");
    fetch(server.admin(Method::GET, &path).header(ACCEPT, accept)).await
}

fn assert_image(fetched: &Fetched, format: ImageFormat, size: (u32, u32)) {
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.header(&CONTENT_TYPE), format.to_mime_type());
    let image = image::load_from_memory_with_format(&fetched.body, format).expect("Decodes");
    assert_eq!(image.dimensions(), size);
}

/// A one-page PDF whose 10 by 300 points page renders 96000 pixels high.
const TALL_PDF: &[u8] = b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
    2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
    3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 10 300] >> endobj\n\
    trailer << /Root 1 0 R >>\n%%EOF\n";

/// Rendered sizes are those of PDFBox 3.0.8 `PDFRenderer.renderImage` at Komga's scale.
#[tokio::test]
async fn pdf_pages_and_covers_are_rendered_as_jpeg() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/pages.pdf", pdf("pages.pdf")),
            ("Series/komga.pdf", pdf("komga.pdf")),
            ("Series/tall.pdf", TALL_PDF.to_vec()),
        ],
    )
    .await;
    let (book_id, _) = book(&pool, "pages").await;

    // Rendered concurrently, each with its own render cache.
    let (first, second, third, fourth) = tokio::join!(
        page(&server, &book_id, "1"),
        page(&server, &book_id, "2"),
        page(&server, &book_id, "3"),
        page(&server, &book_id, "4"),
    );
    assert_image(&first, ImageFormat::Jpeg, (3200, 4525));
    assert_image(&second, ImageFormat::Jpeg, (4791, 3200));
    // Rotated by 90 degrees.
    assert_image(&third, ImageFormat::Jpeg, (3200, 4266));
    assert_image(&fourth, ImageFormat::Jpeg, (3200, 4800));

    assert_image(
        &page(&server, &book_id, "1?convert=png").await,
        ImageFormat::Png,
        (3200, 4525),
    );
    assert_image(
        &page(&server, &book_id, "3/thumbnail").await,
        ImageFormat::Jpeg,
        (225, 300),
    );
    let beyond = page(&server, &book_id, "5").await;
    assert_eq!(beyond.status, StatusCode::BAD_REQUEST);

    // The cover is the first page; a page too large to render fails its read and its cover.
    wait_for_generations(directory.path()).await;
    let (komga_id, _) = book(&pool, "komga").await;
    let cover =
        fetch(server.admin(Method::GET, &format!("/api/v1/books/{komga_id}/thumbnail"))).await;
    assert_jpeg(&cover, (300, 300));
    let (tall_id, _) = book(&pool, "tall").await;
    assert_eq!(
        media(&pool, &tall_id).await,
        ("READY".to_owned(), Some(PDF.to_owned()), None, 1)
    );
    let tall = page(&server, &tall_id, "1").await;
    assert_eq!(tall.status, StatusCode::INTERNAL_SERVER_ERROR);
    let tall_cover =
        fetch(server.admin(Method::GET, &format!("/api/v1/books/{tall_id}/thumbnail"))).await;
    assert_eq!(tall_cover.status, StatusCode::NOT_FOUND);

    std::fs::remove_file(directory.path().join("comics/Series/komga.pdf")).expect("Removed book");
    // Spring writes error bodies only as JSON.
    let gone = page(&server, &komga_id, "1").await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert!(gone.body.is_empty());
    let gone = page_accepting(&server, &komga_id, "1", "application/json").await;
    assert_eq!(gone.message(), "File not found, it may have moved");
    server.stop().await;
}

/// The rounded size of the single page of a raw PDF page response, as a PDF reader sees it.
fn raw_page_size(fetched: &Fetched, file_name: &str) -> (f64, f64) {
    assert_eq!(fetched.status, StatusCode::OK, "{file_name}");
    assert_eq!(fetched.header(&CONTENT_TYPE), PDF);
    assert_eq!(
        fetched.header(&CONTENT_DISPOSITION),
        format!("inline; filename=\"=?UTF-8?Q?{file_name}?=\"; filename*=UTF-8''{file_name}")
    );
    let document = PdfDocument::from_bytes(Arc::new(fetched.body.clone())).expect("PDF page");
    let sizes = document.page_sizes();
    let [PageSize { width, height }] = sizes[..] else {
        panic!("{file_name} has {} pages", sizes.len());
    };
    (width.round(), height.round())
}

#[tokio::test]
async fn pdf_pages_are_served_raw_when_asked_or_negotiated() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/pages.pdf", pdf("pages.pdf"))]).await;
    let (book_id, _) = book(&pool, "pages").await;
    let raw = |path: &'static str| {
        page_accepting(
            &server,
            &book_id,
            path,
            "image/jpeg, application/json;q=0.1",
        )
    };

    assert_eq!(
        raw_page_size(&raw("2/raw").await, "pages-2.pdf"),
        (300.0, 201.0)
    );
    for number in ["0/raw", "5/raw"] {
        let beyond = raw(number).await;
        assert_eq!(beyond.status, StatusCode::BAD_REQUEST, "{number}");
        assert_eq!(beyond.message(), "Page number does not exist");
    }

    // Spring `isCompatibleWith` and `sortBySpecificity` decide between the raw page and the
    // image; a raw page ignores `convert`.
    for (path, accept, raw) in [
        ("1", "*/*", true),
        ("1", "application/pdf, image/*", true),
        ("1", "image/*;q=0.5, application/*", true),
        ("1?convert=gif", "application/pdf", true),
        ("1", "image/jpeg, application/pdf", false),
        ("1", "application/pdf;q=0.5, image/png", false),
        ("1?contentNegotiation=false", "application/pdf", false),
    ] {
        let fetched = page_accepting(&server, &book_id, path, accept).await;
        if raw {
            assert_eq!(raw_page_size(&fetched, "pages-1.pdf"), (595.0, 842.0));
        } else {
            assert_image(&fetched, ImageFormat::Jpeg, (3200, 4525));
        }
    }

    // A new version of the file is read again, even before a rescan.
    let path = directory.path().join("comics/Series/pages.pdf");
    fs::write(&path, pdf("komga.pdf")).expect("Replaced book");
    let modified = SystemTime::now() + Duration::from_secs(10);
    File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_modified(modified))
        .expect("New modification time");
    assert_eq!(
        raw_page_size(&raw("1/raw").await, "pages-1.pdf"),
        (512.0, 512.0)
    );
    assert_eq!(raw("2/raw").await.status, StatusCode::BAD_REQUEST);
    fs::remove_file(&path).expect("Removed book");
    let gone = raw("1/raw").await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
    assert_eq!(gone.message(), "File not found, it may have moved");
    server.stop().await;
}
