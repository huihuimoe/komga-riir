//! Book and series metadata from the package document of an EPUB (`EpubMetadataProvider`).

use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{PNG, books, library, rescan_deep, series, write};
use crate::libraries::{admin_json, route};
use crate::support::TestServer;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;

const CONTAINER: &str =
    r#"<container><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#;

/// An EPUB of one page and a cover whose package document has `title` and the collection `collection`.
fn epub(title: &str, collection: &str) -> Vec<u8> {
    let opf = format!(
        r##"<package xmlns="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <metadata>
    <dc:title>{title}</dc:title>
    <dc:description>&lt;p&gt;Tom &amp;amp; &lt;b&gt;Jerry&lt;/b&gt;&lt;/p&gt;</dc:description>
    <dc:date>1999-07-31T16:00:00+00:00</dc:date>
    <dc:creator id="c1">Ulf Blanck</dc:creator>
    <dc:creator id="c2">The Editor</dc:creator>
    <dc:creator id="c3" opf:role="ill">Ann Art</dc:creator>
    <meta refines="#c2" property="role" scheme="marc:relators">edt</meta>
    <dc:identifier>AMAZON:3440077934</dc:identifier>
    <dc:identifier>ISBN:080442957X</dc:identifier>
    <dc:publisher>Kosmos</dc:publisher>
    <dc:language>DE</dc:language>
    <dc:subject>Mystery</dc:subject>
    <dc:subject>Kids</dc:subject>
    <meta property="belongs-to-collection" id="s">{collection}</meta>
    <meta refines="#s" property="group-position">2.5</meta>
  </metadata>
  <manifest>
    <item id="p" href="p.xhtml" media-type="application/xhtml+xml"/>
    <item id="c" href="cover.png" media-type="image/png" properties="cover-image"/>
  </manifest>
  <spine page-progression-direction="rtl"><itemref idref="p"/></spine>
</package>"##
    );
    let page = "<html><body><p>Page</p></body></html>";
    let cover = cbz::fixture(PNG);
    cbz::zip(&[
        ("mimetype", STORED, b"application/epub+zip"),
        ("META-INF/container.xml", STORED, CONTAINER.as_bytes()),
        ("OPS/book.opf", STORED, opf.as_bytes()),
        ("OPS/p.xhtml", STORED, page.as_bytes()),
        ("OPS/cover.png", STORED, &cover),
    ])
}

fn expected_book() -> books::Fields {
    (
        "Panik".to_owned(),
        "Tom & Jerry".to_owned(),
        "2.5".to_owned(),
        2.5,
        Some("1999-07-31".to_owned()),
        // `080442957X` as ISBN-13.
        "9780804429573".to_owned(),
    )
}

fn expected_series(title: &str) -> (series::Fields, Vec<String>) {
    (
        (
            title.to_owned(),
            title.to_owned(),
            Some("RIGHT_TO_LEFT".to_owned()),
            "Kosmos".to_owned(),
            None,
            "de".to_owned(),
            None,
        ),
        vec!["kids".to_owned(), "mystery".to_owned()],
    )
}

#[tokio::test]
async fn an_epub_takes_its_package_metadata_unless_the_library_imports_none() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    write(
        directory.path(),
        "Folder/book.epub",
        &epub("Panik", "Die drei ??? Kids"),
    );
    let (server, library, pool) = library(directory.path()).await;
    let (book_id, series_id) = book(&pool, "book").await;
    assert_eq!(books::fields(&pool, &book_id).await, expected_book());
    assert_eq!(
        books::pairs(&pool, "BOOK_METADATA_AUTHOR", "NAME, ROLE", &book_id).await,
        [
            books::pair("Ulf Blanck", "writer"),
            books::pair("The Editor", "editor"),
            books::pair("Ann Art", "penciller"),
        ]
    );
    assert_eq!(
        series::fields(&pool, &series_id).await,
        expected_series("Die drei ??? Kids")
    );

    disable_epub_import(&server, directory.path(), &library).await;
    let (title, summary, _, _, release_date, isbn) = expected_book();
    assert_eq!(
        books::fields(&pool, &book_id).await,
        (title, summary, "1".to_owned(), 1.0, release_date, isbn),
        "the library imports no EPUB book metadata; the sort renumbers the changed book"
    );
    assert_eq!(
        series::fields(&pool, &series_id).await,
        expected_series("Die drei ??? Kids"),
        "the library imports no EPUB series metadata"
    );
    server.stop().await;
}

/// Turns both EPUB imports off, then rescans a changed copy of the book.
async fn disable_epub_import(server: &TestServer, directory: &Path, library: &Value) {
    admin_json(
        server,
        Method::PATCH,
        &route(library),
        Some(&json!({"importEpubBook": false, "importEpubSeries": false})),
        StatusCode::NO_CONTENT,
    )
    .await;
    write(directory, "Folder/book.epub", &epub("Changed", "Other"));
    rescan_deep(server, directory, library).await;
}
