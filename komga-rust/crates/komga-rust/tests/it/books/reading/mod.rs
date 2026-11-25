//! Page list, page, and raw page of analyzed ZIP books, and book file downloads.

pub(crate) mod access;
mod conversion;
mod download;

use std::io::Write;

use flate2::Compression;
use flate2::write::DeflateEncoder;

use reqwest::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, HeaderMap,
    IF_MODIFIED_SINCE, IF_UNMODIFIED_SINCE, LAST_MODIFIED,
};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde_json::{Value, json};

use super::{COMIC_INFO, GIF, JPEG, PNG, library_with_books};
use crate::support::cbz::{self, DEFLATED, STORED};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

const CACHE_PRIVATE: &str = "max-age=0, must-revalidate, private";
const EPOCH: &str = "Thu, 01 Jan 1970 00:00:00 GMT";

/// Pages out of natural order around a non-page member.
pub(crate) fn pages_book() -> Vec<u8> {
    cbz::zip(&[
        ("p2.jpg", STORED, &cbz::fixture(JPEG)),
        ("p10.png", STORED, &cbz::fixture(PNG)),
        ("ComicInfo.xml", STORED, &cbz::fixture(COMIC_INFO)),
        ("P1.gif", STORED, &cbz::fixture(GIF)),
    ])
}

pub(crate) struct Fetched {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

impl Fetched {
    pub(crate) fn header(&self, name: &reqwest::header::HeaderName) -> &str {
        self.headers
            .get(name)
            .map_or("", |value| value.to_str().expect("ASCII header"))
    }

    pub(crate) fn message(&self) -> String {
        let body: Value = serde_json::from_slice(&self.body).expect("JSON error body");
        body["message"].as_str().expect("Error message").to_owned()
    }
}

pub(crate) async fn fetch(request: RequestBuilder) -> Fetched {
    let response = request.send().await.expect("Request");
    Fetched {
        status: response.status(),
        headers: response.headers().clone(),
        body: response.bytes().await.expect("Body").to_vec(),
    }
}

/// A server whose library holds `Series/Série é.cbz`, analyzed, and that book's page route.
async fn server_with_pages_book(directory: &std::path::Path) -> (TestServer, String) {
    let (server, _, pool) =
        library_with_books(directory, &[("Series/Série é.cbz", pages_book())]).await;
    let (book_id, _) = book(&pool, "Série é").await;
    (server, format!("/api/v1/books/{book_id}/pages"))
}

/// Spring's `Content-Disposition` for page `number` of `Série é`.
fn disposition(file: &str) -> String {
    format!(
        "inline; filename=\"=?UTF-8?Q?S=C3=A9rie_=C3=A9-{file}?=\"; \
         filename*=UTF-8''S%C3%A9rie%20%C3%A9-{file}"
    )
}

#[tokio::test]
async fn pages_are_listed_and_served_as_stored() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pages) = server_with_pages_book(directory.path()).await;

    let response = server
        .admin(Method::GET, &pages)
        .send()
        .await
        .expect("List");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        openapi::checked(Method::GET, response).await,
        json!([
            {"number": 1, "fileName": "P1.gif", "mediaType": "image/gif", "width": 48,
             "height": 20, "sizeBytes": 1607, "size": "1.6 KiB"},
            {"number": 2, "fileName": "p2.jpg", "mediaType": "image/jpeg", "width": 3,
             "height": 2, "sizeBytes": 412, "size": "412 B"},
            {"number": 3, "fileName": "p10.png", "mediaType": "image/png", "width": 48,
             "height": 20, "sizeBytes": 251, "size": "251 B"},
        ])
    );

    // A page already in the requested format is served as stored.
    for (route, fixture, media_type, file) in [
        ("/1", GIF, "image/gif", "1.gif"),
        ("/0?zero_based=true", GIF, "image/gif", "1.gif"),
        ("/2?convert=JPEG", JPEG, "image/jpeg", "2.jpg"),
        ("/%203?convert=png&zero_based=", PNG, "image/png", "3.png"),
        ("/0x3?convert=", PNG, "image/png", "3.png"),
    ] {
        let page = fetch(server.admin(Method::GET, &format!("{pages}{route}"))).await;
        assert_eq!(page.status, StatusCode::OK, "{route}");
        assert!(page.body == cbz::fixture(fixture), "{route}: page bytes");
        assert_eq!(page.header(&CONTENT_TYPE), media_type, "{route}");
        assert_eq!(
            page.header(&CONTENT_DISPOSITION),
            disposition(file),
            "{route}"
        );
        let csp = "default-src 'none'; style-src 'unsafe-inline'; sandbox";
        assert_eq!(page.header(&CONTENT_SECURITY_POLICY), csp, "{route}");
        assert_eq!(page.header(&CACHE_CONTROL), CACHE_PRIVATE, "{route}");
        assert!(page.header(&LAST_MODIFIED).ends_with(" GMT"), "{route}");
    }

    let mismatch = |name: &str, kind: &str, cause: &str| {
        format!(
            "Method parameter '{name}': Failed to convert value of type 'java.lang.String' to \
             required type '{kind}'; {cause}"
        )
    };
    for (route, message) in [
        ("/0", "Page number does not exist".to_owned()),
        ("/4", "Page number does not exist".to_owned()),
        (
            "/3?zero_based=true",
            "Page number does not exist".to_owned(),
        ),
        // The format is checked before the page number.
        (
            "/9?convert=bmp",
            "Invalid conversion format: bmp".to_owned(),
        ),
        (
            "/1?convert=jpeg&convert=png",
            "Invalid conversion format: jpeg,png".to_owned(),
        ),
        (
            "/one",
            mismatch("pageNumber", "int", "For input string: \"one\""),
        ),
        (
            "/1?zero_based=maybe",
            mismatch("zero_based", "boolean", "Invalid boolean value [maybe]"),
        ),
        (
            "/1?contentNegotiation=maybe",
            mismatch(
                "contentNegotiation",
                "boolean",
                "Invalid boolean value [maybe]",
            ),
        ),
        (
            "/1/raw",
            "Extractor does not support raw extraction of pages".to_owned(),
        ),
        (
            "/one/raw",
            mismatch("pageNumber", "int", "For input string: \"one\""),
        ),
    ] {
        let response = fetch(server.admin(Method::GET, &format!("{pages}{route}"))).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "{route}");
        assert_eq!(response.message(), message, "{route}");
    }
    server.stop().await;
}

#[tokio::test]
async fn conditional_requests_are_answered_before_content_restrictions() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pages) = server_with_pages_book(directory.path()).await;
    let first = fetch(server.admin(Method::GET, &format!("{pages}/1"))).await;
    let last_modified = first.header(&LAST_MODIFIED).to_owned();

    for (route, header, date, status) in [
        (
            "/1",
            IF_MODIFIED_SINCE,
            last_modified.as_str(),
            StatusCode::NOT_MODIFIED,
        ),
        ("/1", IF_MODIFIED_SINCE, EPOCH, StatusCode::OK),
        // Spring's failed If-Unmodified-Since precondition still reaches the 304 branch.
        ("/1", IF_UNMODIFIED_SINCE, EPOCH, StatusCode::NOT_MODIFIED),
        (
            "/1",
            IF_UNMODIFIED_SINCE,
            last_modified.as_str(),
            StatusCode::OK,
        ),
        (
            "/1/raw",
            IF_MODIFIED_SINCE,
            last_modified.as_str(),
            StatusCode::NOT_MODIFIED,
        ),
        ("/1/raw", IF_MODIFIED_SINCE, EPOCH, StatusCode::BAD_REQUEST),
    ] {
        let request = server.admin(Method::GET, &format!("{pages}{route}"));
        let response = fetch(request.header(&header, date)).await;
        assert_eq!(response.status, status, "{route} {header}: {date}");
        if status == StatusCode::NOT_MODIFIED {
            assert!(response.body.is_empty(), "{route}");
            assert_eq!(response.header(&CACHE_CONTROL), CACHE_PRIVATE, "{route}");
            assert_eq!(response.header(&LAST_MODIFIED), last_modified, "{route}");
        }
    }

    // As in Kotlin, the date check precedes the library check.
    let outsider = access::create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": ["PAGE_STREAMING"], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    let page = format!("{pages}/1");
    let conditional = outsider
        .get(&server, &page)
        .header(IF_MODIFIED_SINCE, &last_modified);
    assert_eq!(fetch(conditional).await.status, StatusCode::NOT_MODIFIED);
    let denied = fetch(outsider.get(&server, &page)).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.message(), "403 FORBIDDEN");
    server.stop().await;
}

/// A PNG page padded with zeros to one byte more than reads whole, deflated, so that
/// the archive holds a small member that decodes past the limit.
fn page_bomb() -> Vec<u8> {
    let mut page = cbz::fixture(PNG);
    page.resize(256 * 1024 * 1024 + 1, 0);
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&page).expect("Deflate in memory");
    encoder.finish().expect("Deflate in memory")
}

#[tokio::test]
async fn pages_are_archive_members_read_within_limits() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (png, gif) = (cbz::fixture(PNG), cbz::fixture(GIF));
    let book_file = cbz::zip(&[
        ("../../escape.png", STORED, &png),
        ("/absolute.gif", STORED, &gif),
        ("z/../../bomb.png", DEFLATED, &page_bomb()),
    ]);
    let (server, _, pool) = library_with_books(directory.path(), &[("book.cbz", book_file)]).await;
    let (book_id, _) = book(&pool, "book").await;
    let pages = format!("/api/v1/books/{book_id}/pages");
    let listed: Value =
        serde_json::from_slice(&fetch(server.admin(Method::GET, &pages)).await.body)
            .expect("Page list");
    let names: Vec<&str> = listed
        .as_array()
        .expect("Pages")
        .iter()
        .map(|page| page["fileName"].as_str().expect("File name"))
        .collect();
    assert_eq!(
        names,
        ["../../escape.png", "/absolute.gif", "z/../../bomb.png"]
    );

    // Names only select members: nothing outside the archive is read.
    for (number, content) in [(1, &png), (2, &gif)] {
        let page = fetch(server.admin(Method::GET, &format!("{pages}/{number}"))).await;
        assert_eq!(page.status, StatusCode::OK, "{number}");
        assert!(page.body == *content, "{number}: page bytes");
    }
    let bomb = fetch(server.admin(Method::GET, &format!("{pages}/3"))).await;
    assert_eq!(bomb.status, StatusCode::INTERNAL_SERVER_ERROR);
    server.stop().await;
}
