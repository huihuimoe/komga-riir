//! EPUB books (`BookAnalyzer.analyzeEpub`) from scan to the stored media, files, and extension,
//! and their covers and DiViNa pages.

use std::io::Read;

use flate2::read::GzDecoder;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::reading::fetch;
use super::thumbnails::assert_jpeg;
use super::{PNG, library_with_books};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::{book, media, pages};
use crate::support::tasks::{GENERATE_BOOK_THUMBNAIL, wait_for_generations};

const EPUB: &str = "application/epub+zip";
const CONTAINER: &str =
    r#"<container><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#;

pub(crate) fn sample(name: &str) -> Vec<u8> {
    cbz::fixture(&format!("komga-archive/tests/fixtures/epub/{name}"))
}

pub(crate) fn epub(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut all: Vec<(&str, u16, &[u8])> = vec![
        ("mimetype", STORED, EPUB.as_bytes()),
        ("META-INF/container.xml", STORED, CONTAINER.as_bytes()),
    ];
    all.extend(members.iter().map(|&(name, data)| (name, STORED, data)));
    cbz::zip(&all)
}

/// Two spine documents showing one image each.
pub(crate) fn divina_book(png: &[u8]) -> Vec<u8> {
    let opf = r#"<package><manifest>
        <item id="p1" href="p1.xhtml" media-type="application/xhtml+xml"/>
        <item id="p2" href="p2.xhtml" media-type="application/xhtml+xml"/>
        <item id="i1" href="1.png" media-type="image/png"/>
        <item id="i2" href="2.png" media-type="image/png"/>
      </manifest><spine><itemref idref="p1"/><itemref idref="p2"/></spine></package>"#;
    let page = |image: &str| format!(r#"<html><body><img src="{image}"/></body></html>"#);
    epub(&[
        ("OPS/book.opf", opf.as_bytes()),
        ("OPS/p1.xhtml", page("1.png").as_bytes()),
        ("OPS/p2.xhtml", page("2.png").as_bytes()),
        ("OPS/1.png", png),
        ("OPS/2.png", png),
    ])
}

/// A KEPUB whose TOC href is malformed and whose stylesheet is missing.
fn damaged_kepub() -> Vec<u8> {
    let opf = r#"<package><manifest>
        <item id="nav" href="nav.xhtml" properties="nav" media-type="application/xhtml+xml"/>
        <item id="a" href="a.xhtml" media-type="application/xhtml+xml"/>
        <item id="css" href="missing.css" media-type="text/css"/>
      </manifest><spine><itemref idref="a"/></spine></package>"#;
    let nav = r#"<html><body><nav epub:type="toc"><ol><li><a href="100%.xhtml">A</a></li></ol></nav>
        <nav epub:type="landmarks"><ol><li><a href="a.xhtml">Start</a></li></ol></nav></body></html>"#;
    let a = r#"<html><body><p><span class="koboSpan" id="kobo.1.1">More than fifteen letters.</span></p></body></html>"#;
    epub(&[
        ("OPS/book.opf", opf.as_bytes()),
        ("OPS/nav.xhtml", nav.as_bytes()),
        ("OPS/a.xhtml", a.as_bytes()),
    ])
}

/// `(FILE_NAME, MEDIA_TYPE, SUB_TYPE, FILE_SIZE)` in insertion order.
async fn files(pool: &SqlitePool, book_id: &str) -> Vec<(String, String, String, i64)> {
    sqlx::query_as(
        "SELECT FILE_NAME, MEDIA_TYPE, SUB_TYPE, FILE_SIZE FROM MEDIA_FILE WHERE BOOK_ID = ? \
         ORDER BY ROWID",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .expect("Stored files")
}

/// `(EPUB_DIVINA_COMPATIBLE, EPUB_IS_KEPUB, EXTENSION_CLASS, decoded extension)`.
async fn epub_media(pool: &SqlitePool, book_id: &str) -> (bool, bool, Option<String>, Value) {
    let (divina, kepub, class, blob): (bool, bool, Option<String>, Option<Vec<u8>>) =
        sqlx::query_as(
            "SELECT EPUB_DIVINA_COMPATIBLE, EPUB_IS_KEPUB, EXTENSION_CLASS, EXTENSION_VALUE_BLOB \
             FROM MEDIA WHERE BOOK_ID = ?",
        )
        .bind(book_id)
        .fetch_one(pool)
        .await
        .expect("Stored media");
    let extension = blob.map_or(Value::Null, |blob| {
        let mut json = String::new();
        GzDecoder::new(blob.as_slice())
            .read_to_string(&mut json)
            .expect("Gzip extension");
        serde_json::from_str(&json).expect("JSON extension")
    });
    (divina, kepub, class, extension)
}

/// `(STATUS, MEDIA_TYPE, COMMENT, PAGE_COUNT)` as `media` reads it.
fn row(
    status: &str,
    media_type: &str,
    comment: Option<&str>,
    page_count: i32,
) -> (String, Option<String>, Option<String>, i32) {
    (
        status.to_owned(),
        Some(media_type.to_owned()),
        comment.map(str::to_owned),
        page_count,
    )
}

#[tokio::test]
async fn a_scanned_sample_epub_is_ready_and_broken_ones_are_errors() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (_server, _, pool) = library_with_books(
        directory.path(),
        &[
            (
                "Series/theft.epub",
                sample("The Incomplete Theft - Ralph Burke.epub"),
            ),
            ("Series/zip.epub", sample("zip-as-epub.epub")),
            ("Series/headless.epub", epub(&[])),
        ],
    )
    .await;

    let theft = book(&pool, "theft").await.0;
    assert_eq!(media(&pool, &theft).await, row("READY", EPUB, None, 14));
    let stored = files(&pool, &theft).await;
    assert_eq!(stored.len(), 8);
    assert_eq!(
        stored[0],
        (
            "titlepage.xhtml".to_owned(),
            "application/xhtml+xml".to_owned(),
            "EPUB_PAGE".to_owned(),
            818
        )
    );
    assert_eq!(
        stored[7],
        (
            "nav.xhtml".to_owned(),
            "application/xhtml+xml".to_owned(),
            "EPUB_ASSET".to_owned(),
            621
        )
    );
    let (divina, kepub, class, extension) = epub_media(&pool, &theft).await;
    assert_eq!(
        (divina, kepub, class.as_deref()),
        (
            false,
            false,
            Some("org.gotson.komga.domain.model.MediaExtensionEpub")
        )
    );
    assert_eq!(
        extension["landmarks"],
        json!([{"title": "Cover", "href": "titlepage.xhtml", "children": []}])
    );
    assert_eq!(extension["isFixedLayout"], json!(false));
    assert_eq!(extension["positions"].as_array().map(Vec::len), Some(35));
    assert_eq!(
        extension["positions"][0],
        json!({
            "href": "titlepage.xhtml",
            "type": "application/xhtml+xml",
            "locations": {"progression": 0.0, "position": 1, "totalProgression": 0.028_571_429},
            "koboSpan": "kobo.1.1",
        })
    );

    assert_eq!(
        media(&pool, &book(&pool, "zip").await.0).await,
        row("ERROR", "application/zip", Some("ERR_1032"), 0),
        "an .epub file without the EPUB mimetype"
    );
    assert_eq!(
        media(&pool, &book(&pool, "headless").await.0).await,
        row("ERROR", EPUB, Some("ERR_1005"), 0),
        "no container"
    );
}

#[tokio::test]
async fn divina_epubs_get_image_pages_and_damaged_parts_are_noted() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let png = cbz::fixture(PNG);
    let (_server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/divina.epub", divina_book(&png)),
            ("Series/damaged.epub", damaged_kepub()),
        ],
    )
    .await;

    let divina_id = book(&pool, "divina").await.0;
    assert_eq!(media(&pool, &divina_id).await, row("READY", EPUB, None, 2));
    let size = i64::try_from(png.len()).ok();
    let page = |name: &str| {
        (
            name.to_owned(),
            "image/png".to_owned(),
            Some(48),
            Some(20),
            String::new(),
            size,
        )
    };
    assert_eq!(
        pages(&pool, &divina_id).await,
        [page("OPS/1.png"), page("OPS/2.png")]
    );
    let (divina, kepub, _, extension) = epub_media(&pool, &divina_id).await;
    assert_eq!((divina, kepub), (true, false));
    assert_eq!(
        extension["isFixedLayout"],
        json!(true),
        "DiViNa books are fixed layout"
    );
    assert_eq!(extension["positions"].as_array().map(Vec::len), Some(2));

    let damaged = book(&pool, "damaged").await.0;
    assert_eq!(
        media(&pool, &damaged).await,
        row(
            "READY",
            EPUB,
            Some("ERR_1035 ERR_1033 [OPS/missing.css]"),
            1
        )
    );
    let (divina, kepub, _, extension) = epub_media(&pool, &damaged).await;
    assert_eq!((divina, kepub), (false, true));
    assert_eq!(extension["toc"], json!([]));
    assert_eq!(extension["landmarks"][0]["href"], json!("OPS/a.xhtml"));
    assert_eq!(
        files(&pool, &damaged).await.len(),
        2,
        "the missing stylesheet is not a file"
    );
}

#[tokio::test]
async fn epub_covers_and_divina_pages_are_served() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let png = cbz::fixture(PNG);
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            (
                "Series/theft.epub",
                sample("The Incomplete Theft - Ralph Burke.epub"),
            ),
            ("Series/divina.epub", divina_book(&png)),
            ("Series/damaged.epub", damaged_kepub()),
        ],
    )
    .await;
    // A book without a cover completes its generation instead of failing it.
    let tasks = wait_for_generations(directory.path()).await;
    assert!(
        tasks.iter().all(|row| row.2 != GENERATE_BOOK_THUMBNAIL),
        "{tasks:?}"
    );
    let thumbnail =
        |id: String| fetch(server.admin(Method::GET, &format!("/api/v1/books/{id}/thumbnail")));
    let page = |id: &str, path: &str| {
        fetch(server.admin(Method::GET, &format!("/api/v1/books/{id}/pages/{path}")))
    };

    // The manifest's cover image, which is no page.
    let theft = book(&pool, "theft").await.0;
    assert_jpeg(&thumbnail(theft.clone()).await, (225, 300));
    let unsupported = page(&theft, "1").await;
    assert_eq!(unsupported.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(page(&theft, "15").await.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        page(&theft, "1/thumbnail").await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );

    // Without a cover image, DiViNa books start with their first page.
    let divina = book(&pool, "divina").await.0;
    assert_jpeg(&thumbnail(divina.clone()).await, (48, 20));
    let second = page(&divina, "2").await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(second.header(&CONTENT_TYPE), "image/png");
    assert_eq!(second.body, png);
    assert_eq!(
        page(&divina, "1?convert=jpeg").await.header(&CONTENT_TYPE),
        "image/jpeg"
    );
    assert_jpeg(&page(&divina, "1/thumbnail").await, (48, 20));
    assert_eq!(page(&divina, "3").await.status, StatusCode::BAD_REQUEST);

    let damaged = book(&pool, "damaged").await.0;
    assert_eq!(
        thumbnail(damaged).await.status,
        StatusCode::NOT_FOUND,
        "no cover image and no DiViNa page"
    );
}
