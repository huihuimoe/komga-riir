//! Members of analyzed EPUB books (`CommonBookController.getBookEpubResource`).

use std::fs;

use reqwest::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, IF_MODIFIED_SINCE, LAST_MODIFIED,
};
use reqwest::{Method, StatusCode};
use serde_json::json;

use super::epub::epub;
use super::reading::access::create_reader;
use super::reading::{Fetched, fetch};
use super::{PNG, library_with_books};
use crate::support::ADMIN_EMAIL;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;

const CSP: &str = "script-src 'none'; object-src 'none';";
const PAGE: &[u8] = b"<html><body><p>Page</p></body></html>";
const CSS: &[u8] = b"p { color: red }";
const FONT: &[u8] = b"\0\x01\0\0font";

/// A page in a subdirectory with a space in its name, a stylesheet, a font, and a member whose
/// declared media type does not parse; without the stylesheet when `with_css` is false.
fn resources_book(with_css: bool) -> Vec<u8> {
    let opf = r#"<package><manifest>
        <item id="a" href="text/a%20b.xhtml" media-type="application/xhtml+xml"/>
        <item id="css" href="style.css" media-type="text/css"/>
        <item id="font" href="fonts/Serif.TTF" media-type="font/ttf"/>
        <item id="data" href="data.bin" media-type="not a type"/>
      </manifest><spine><itemref idref="a"/></spine></package>"#;
    let mut members: Vec<(&str, &[u8])> = vec![
        ("OPS/book.opf", opf.as_bytes()),
        ("OPS/text/a b.xhtml", PAGE),
        ("OPS/fonts/Serif.TTF", FONT),
        ("OPS/data.bin", b"data"),
    ];
    if with_css {
        members.push(("OPS/style.css", CSS));
    }
    epub(&members)
}

fn assert_served(response: &Fetched, body: &[u8], content_type: &str, disposition: &str) {
    assert_eq!(response.status, StatusCode::OK, "{disposition}");
    assert_eq!(response.body, body, "{disposition}");
    assert_eq!(
        response.header(&CONTENT_TYPE),
        content_type,
        "{disposition}"
    );
    assert_eq!(response.header(&CONTENT_DISPOSITION), disposition);
    assert_eq!(
        response.header(&"content-security-policy".parse().expect("Name")),
        CSP
    );
}

#[tokio::test]
async fn epub_resources_are_the_analyzed_files() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let png = cbz::fixture(PNG);
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/book.epub", resources_book(true)),
            ("Series/comic.cbz", cbz::zip(&[("p.png", STORED, &png)])),
        ],
    )
    .await;
    let id = book(&pool, "book").await.0;
    let get = |path: &str| fetch(server.admin(Method::GET, &format!("/api/v1/books/{id}/{path}")));

    let page = get("resource/OPS/text/a%20b.xhtml").await;
    // OPDS 2 serves them too, but only to authenticated clients.
    let opds = format!("/opds/v2/books/{id}/resource/OPS/text/a%20b.xhtml");
    assert!(fetch(server.admin(Method::GET, &opds)).await.body == page.body);
    let anonymous = fetch(server.request(Method::GET, &opds)).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_served(
        &page,
        PAGE,
        "application/xhtml+xml",
        "inline; filename=\"=?UTF-8?Q?a_b.xhtml?=\"; filename*=UTF-8''a%20b.xhtml",
    );
    assert_eq!(
        page.header(&CACHE_CONTROL),
        "max-age=0, must-revalidate, private"
    );
    assert_served(
        &get("resource/OPS/data.bin").await,
        b"data",
        "application/octet-stream",
        "inline; filename=\"=?UTF-8?Q?data.bin?=\"; filename*=UTF-8''data.bin",
    );

    // Only the book's files are served, whatever else the archive holds.
    for path in [
        "resource/META-INF/container.xml",
        "resource/OPS/text/..%2Fstyle.css",
        "resource/",
        "resource",
    ] {
        assert_eq!(get(path).await.status, StatusCode::NOT_FOUND, "{path}");
    }

    let last_modified = page.header(&LAST_MODIFIED).to_owned();
    let conditional = server
        .admin(
            Method::GET,
            &format!("/api/v1/books/{id}/resource/OPS/style.css"),
        )
        .header(IF_MODIFIED_SINCE, &last_modified);
    let not_modified = fetch(conditional).await;
    assert_eq!(not_modified.status, StatusCode::NOT_MODIFIED);
    assert_eq!(
        not_modified.header(&"content-security-policy".parse().expect("Name")),
        CSP
    );

    let comic = book(&pool, "comic").await.0;
    let path = format!("/api/v1/books/{comic}/resource/p.png");
    let other_profile = fetch(server.admin(Method::GET, &path)).await;
    assert_eq!(other_profile.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        other_profile.message(),
        "Book media type 'application/zip' not compatible with requested profile"
    );
    let unknown = fetch(server.admin(Method::GET, "/api/v1/books/0/resource/OPS/style.css"));
    assert_eq!(unknown.await.status, StatusCode::NOT_FOUND);

    // A member the file lost since the analysis.
    let file = directory.path().join("comics/Series/book.epub");
    fs::write(file, resources_book(false)).expect("Book file");
    assert_eq!(
        get("resource/OPS/style.css").await.status,
        StatusCode::NOT_FOUND
    );
    server.stop().await;
}

#[tokio::test]
async fn fonts_are_public_and_other_resources_need_a_visible_book() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[("Series/book.epub", resources_book(true))],
    )
    .await;
    let id = book(&pool, "book").await.0;
    let font = format!("/api/v1/books/{id}/resource/OPS/fonts/Serif.TTF");
    let css = format!("/api/v1/books/{id}/resource/OPS/style.css");
    let anonymous = |path: &str| fetch(server.request(Method::GET, path));

    let disposition = "inline; filename=\"=?UTF-8?Q?Serif.TTF?=\"; filename*=UTF-8''Serif.TTF";
    assert_served(&anonymous(&font).await, FONT, "font/ttf", disposition);
    for path in [css.clone(), format!("/api/v1/books/{id}/resource")] {
        let denied = anonymous(&path).await;
        assert_eq!(denied.status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(denied.message(), "401 UNAUTHORIZED", "{path}");
    }
    let unknown = anonymous("/api/v1/books/0/resource/a.woff").await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    let wrong_password = server
        .request(Method::GET, &font)
        .basic_auth(ADMIN_EMAIL, Some("wrong"));
    assert_eq!(fetch(wrong_password).await.status, StatusCode::UNAUTHORIZED);

    // Fonts skip the content restrictions.
    let outsider = create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    let denied = fetch(outsider.get(&server, &css)).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(denied.message(), "403 FORBIDDEN");
    assert_served(
        &fetch(outsider.get(&server, &font)).await,
        FONT,
        "font/ttf",
        disposition,
    );
    server.stop().await;
}
