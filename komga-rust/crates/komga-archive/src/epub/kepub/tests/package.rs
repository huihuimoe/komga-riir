use std::io::{Cursor, Read};

use jiff::civil::date;
use komga_test_support::TestDir;

use super::super::transform_opf;
use crate::epub::EpubPackage;
use crate::epub::tests::{CONTAINER, write_zip};
use crate::zip::ZipArchive;

const LONG_TEXT: &str = "<p>Once upon a time there lived three little pigs whose mother sent them \
     out into the world to seek their fortunes with nothing but their wits and some straw.</p>";

fn convert(members: &[(&str, &str)]) -> ZipArchive<Cursor<Vec<u8>>> {
    let dir = TestDir::new().unwrap();
    let path = write_zip(&dir, members);
    let mut kepub = Vec::new();
    EpubPackage::open(&path)
        .unwrap()
        .write_kepub(&mut kepub, date(2024, 1, 2).at(3, 4, 5, 0))
        .unwrap();
    // EPUB readers find the stored `mimetype` at a fixed offset.
    assert_eq!(&kepub[30..58], b"mimetypeapplication/epub+zip");
    ZipArchive::new(Cursor::new(kepub)).unwrap()
}

fn read(zip: &mut ZipArchive<Cursor<Vec<u8>>>, name: &str) -> String {
    let index = zip
        .entries()
        .iter()
        .position(|entry| entry.name() == name)
        .unwrap_or_else(|| panic!("{name} is in the KEPUB"));
    let mut content = String::new();
    zip.entry_reader(index)
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    content
}

fn names(zip: &ZipArchive<Cursor<Vec<u8>>>) -> Vec<&str> {
    zip.entries().iter().map(|entry| entry.name()).collect()
}

#[test]
fn packages_are_repackaged_with_kobo_spans_and_without_leftovers() {
    let opf = r#"<package xmlns="http://www.idpf.org/2007/opf"><manifest>
<item id="text" href="Text/Chapter.XHTML" media-type="application/xhtml+xml"/>
<item id="style" href="style.css" media-type="text/css"/>
</manifest><spine><itemref idref="text"/></spine></package>"#;
    let chapter = format!("<html><body>{LONG_TEXT}</body></html>");
    let mut kepub = convert(&[
        ("mimetype", "application/epub+zip\n"),
        ("META-INF/", ""),
        ("META-INF/container.xml", CONTAINER),
        ("OPS/book.opf", opf),
        ("OPS/text/chapter.xhtml", &chapter),
        ("OPS/style.css", "p { color: red }"),
        ("__MACOSX/OPS/._style.css", "junk"),
        ("OPS/Thumbs.db", "junk"),
    ]);
    assert_eq!(
        names(&kepub),
        [
            "mimetype",
            "META-INF/container.xml",
            "OPS/book.opf",
            "OPS/text/chapter.xhtml",
            "OPS/style.css"
        ]
    );
    assert!(
        read(&mut kepub, "OPS/text/chapter.xhtml")
            .contains(r#"<span class="koboSpan" id="kobo.1.1">"#),
        "content documents match their manifest entry case-insensitively"
    );
    assert_eq!(read(&mut kepub, "OPS/style.css"), "p { color: red }");
    assert_eq!(read(&mut kepub, "OPS/book.opf"), opf);
}

#[test]
fn package_document_marks_the_cover_and_drops_calibre_leftovers() {
    let opf = r#"<?xml version="1.0"?>
<opf:package xmlns:opf="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <opf:metadata>
    <dc:contributor opf:role="bkp">calibre (7.0)</dc:contributor>
    <dc:creator opf:role="aut">Author &amp; Co</dc:creator>
    <opf:meta name="calibre:timestamp" content="2024-01-01"/>
    <opf:meta name="cover" content="img"/>
  </opf:metadata>
  <opf:manifest>
    <opf:item id="img" href="cover.jpg" media-type="image/jpeg" properties="old"/>
  </opf:manifest>
  <opf:spine/>
</opf:package>"#;
    let expected = opf
        .replace(
            r#"<dc:contributor opf:role="bkp">calibre (7.0)</dc:contributor>"#,
            "",
        )
        .replace(
            r#"<opf:meta name="calibre:timestamp" content="2024-01-01"/>"#,
            "",
        )
        .replace(r#"properties="old""#, r#"properties="cover-image""#);
    let transformed = transform_opf(opf.as_bytes(), "img", false).unwrap();
    assert_eq!(String::from_utf8(transformed).unwrap(), expected);
}

#[test]
fn a_dummy_title_page_precedes_a_first_page_full_of_text() {
    let opf = r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest>
<opf:item id="nav" href="nav.xhtml" media-type="application/xhtml+xml"/>
<opf:item id="one" href="one.xhtml" media-type="application/xhtml+xml"/>
</opf:manifest><opf:spine><opf:itemref idref="nav" linear="no"/><opf:itemref idref="one"/></opf:spine></opf:package>"#;
    let one = format!("<html><body>{LONG_TEXT}</body></html>");
    let mut kepub = convert(&[
        ("META-INF/container.xml", CONTAINER),
        ("OPS/book.opf", opf),
        ("OPS/nav.xhtml", "<html/>"),
        ("OPS/one.xhtml", &one),
    ]);
    assert_eq!(
        read(&mut kepub, "OPS/book.opf"),
        r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf"><opf:manifest>
<opf:item id="nav" href="nav.xhtml" media-type="application/xhtml+xml"/>
<opf:item id="one" href="one.xhtml" media-type="application/xhtml+xml"/>
<opf:item id="kepubify-titlepage-dummy" href="kepubify-titlepage-dummy.xhtml" media-type="application/xhtml+xml"/></opf:manifest><opf:spine><opf:itemref idref="kepubify-titlepage-dummy"/><opf:itemref idref="nav" linear="no"/><opf:itemref idref="one"/></opf:spine></opf:package>"#
    );
    assert!(
        read(&mut kepub, "OPS/kepubify-titlepage-dummy.xhtml")
            .contains("Page intentionally left blank by kepubify."),
    );
}

#[test]
fn title_pages_and_pictures_need_no_dummy() {
    for (href, body) in [
        ("title.xhtml", LONG_TEXT.to_owned()),
        (
            "one.xhtml",
            r#"<div><img src="cover.jpg"/></div><p>Short</p>"#.to_owned(),
        ),
    ] {
        let opf = format!(
            r#"<package><manifest><item id="one" href="{href}" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="one"/></spine></package>"#
        );
        let page = format!("<html><body>{body}</body></html>");
        let kepub = convert(&[
            ("META-INF/container.xml", CONTAINER),
            ("OPS/book.opf", &opf),
            (&format!("OPS/{href}"), &page),
        ]);
        assert_eq!(names(&kepub).len(), 4, "{href} gets no dummy title page");
    }
}
