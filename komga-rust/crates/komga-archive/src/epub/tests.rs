//! Expected values are those of Kotlin `NavTest`, `NcxTest`, and `OpfTest` on the same fixtures,
//! and of `EpubExtractor` on Komga's sample books.

use std::fs;
use std::path::{Path, PathBuf};

use komga_test_support::TestDir;
use komga_types::{EpubTocEntry, MediaFileSubType};

use super::navigation::{self, Epub2Nav, Epub3Nav};
use super::{EpubError, EpubPackage, EpubResource, is_epub, xml};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/epub")
        .join(name)
}

fn entry(title: &str, href: Option<&str>, children: Vec<EpubTocEntry>) -> EpubTocEntry {
    EpubTocEntry {
        title: title.to_owned(),
        href: href.map(str::to_owned),
        children,
    }
}

fn leaf(title: &str, href: &str) -> EpubTocEntry {
    entry(title, Some(href), Vec::new())
}

fn prefixed(prefix: &str, entries: &[(&str, &str)]) -> Vec<EpubTocEntry> {
    entries
        .iter()
        .map(|(title, href)| leaf(title, &format!("{prefix}{href}")))
        .collect()
}

fn nav_toc(prefix: &str) -> Vec<EpubTocEntry> {
    let mut toc = prefixed(
        prefix,
        &[
            ("Cover", "cover.xhtml"),
            ("Title Page", "titlepage.xhtml"),
            ("Copyright", "copyright.xhtml"),
            ("Table of Contents", "toc.xhtml"),
        ],
    );
    toc.push(entry("An unlinked heading", None, Vec::new()));
    toc.push(entry(
        "Introduction",
        Some(&format!("{prefix}introduction.xhtml")),
        prefixed(
            prefix,
            &[
                ("Spring", "chapter 001.xhtml"),
                ("Summer", "chapter 027.xhtml"),
                ("Fall", "chapter053.xhtml#what:why"),
                ("Winter", "chapter 079.xhtml"),
            ],
        ),
    ));
    toc.extend(prefixed(
        prefix,
        &[("Acknowledgments", "acknowledgements.xhtml")],
    ));
    toc
}

#[test]
fn nav_sections_match_kotlin() {
    let content = fs::read(fixture("nav.xhtml")).unwrap();
    for prefix in ["", "PREFIX/"] {
        let path = format!("{prefix}nav.xhtml");
        let nav = |nav_type| navigation::process_nav(&content, &path, nav_type).unwrap();
        assert_eq!(nav(Epub3Nav::Toc), nav_toc(prefix));
        assert_eq!(
            nav(Epub3Nav::Landmarks),
            prefixed(
                prefix,
                &[
                    ("Begin Reading", "cover.xhtml#coverimage"),
                    ("Table of Contents", "toc.xhtml"),
                ]
            )
        );
        assert_eq!(
            nav(Epub3Nav::PageList),
            prefixed(
                prefix,
                &[
                    ("Cover Page", "xhtml/cover.xhtml"),
                    ("iii", "xhtml/title.xhtml#pg_iii"),
                    ("1", "xhtml/chapter1.xhtml#pg_1"),
                    ("2", "xhtml/chapter1.xhtml#pg_2"),
                    ("107", "xhtml/acknowledgments.xhtml#pg_107"),
                    ("ii", "xhtml/adcard.xhtml#pg_ii"),
                    ("109", "xhtml/abouttheauthor.xhtml#pg_109"),
                    ("iv", "xhtml/copyright.xhtml#pg_iv"),
                ]
            )
        );
    }
}

fn ncx_toc(prefix: &str) -> Vec<EpubTocEntry> {
    let app = "Text/Mart_9780553897852_epub_app_r1.htm";
    let nested = "Text/Mart_9780553897852 epub_app_r1.htm";
    let mut appendix = prefixed(
        prefix,
        &[
            ("THE KINGS AND THEIR COURTS", &format!("{app}#apps01.00")),
            ("THE KING ON THE IRON THRONE", &format!("{app}#apps01.01")),
            ("THE KING IN THE NARROW SEA", &format!("{app}#apps01.02")),
            ("THE KING IN HIGHGARDEN", &format!("{app}#apps01.03")),
            ("THE KING IN THE NORTH", &format!("{app}#apps01.04")),
        ],
    );
    appendix.push(entry(
        "THE QUEEN ACROSS THE WATER",
        Some(&format!("{prefix}{app}#apps01.05")),
        prefixed(
            prefix,
            &[
                ("Another level", &format!("{nested}#apps01.06")),
                ("Yet another level", &format!("{nested}#apps01.07")),
            ],
        ),
    ));
    let mut toc = prefixed(
        prefix,
        &[
            ("COVER", "Text/Mart_9780553897852_epub_cvi_r1.htm#b02-cvi"),
            ("BRAN", "Text/Mart_9780553897852_epub_c69_r1.htm"),
        ],
    );
    toc.push(entry("APPENDIX", Some(&format!("{prefix}{app}")), appendix));
    toc.extend(prefixed(
        prefix,
        &[
            ("ACKNOWLEDGMENTS", "Text/Mart_9780553897852_epub_ack_r1.htm"),
            (
                "Omake 1: Ichika\u{2019}s Q&A",
                "Text/First Omake - Ichika\u{2019}s Q&A.xhtml",
            ),
        ],
    ));
    toc
}

#[test]
fn ncx_sections_match_kotlin() {
    let content = fs::read(fixture("toc.ncx")).unwrap();
    for prefix in ["", "PREFIX/"] {
        let path = format!("{prefix}toc.ncx");
        let ncx = |nav_type| navigation::process_ncx(&content, &path, nav_type).unwrap();
        assert_eq!(ncx(Epub2Nav::Toc), ncx_toc(prefix));
        assert_eq!(
            ncx(Epub2Nav::PageList),
            prefixed(
                prefix,
                &[
                    ("Cover Page", "xhtml/cover.xhtml"),
                    ("iii", "xhtml/title.xhtml#pg_iii"),
                    ("v", "xhtml/dedication.xhtml#pg_v"),
                    ("vii", "xhtml/formoreinformation.xhtml#pg_vii"),
                    ("viii", "xhtml/formoreinformation.xhtml#pg_viii"),
                    ("ix", "xhtml/formoreinformation.xhtml#pg_ix"),
                    ("x", "xhtml/formoreinformation.xhtml#pg_x"),
                    ("xi", "xhtml/formoreinformation.xhtml#pg_xi"),
                    ("1", "xhtml/chapter1.xhtml#pg_1"),
                    ("2", "xhtml/chapter1.xhtml#pg_2"),
                    ("3", "xhtml/chapter1.xhtml#pg_3"),
                ]
            )
        );
    }
}

#[test]
fn opf_guide_matches_kotlin() {
    let opf = xml::parse(&fs::read(fixture("clash.opf")).unwrap());
    for (dir, prefix) in [(None, ""), (Some("PREFIX"), "PREFIX/")] {
        assert_eq!(
            navigation::process_opf_guide(&opf, dir).unwrap(),
            prefixed(
                prefix,
                &[
                    (
                        "Table Of Contents",
                        "Text/Mart_9780553897852_epub_toc_r1.htm"
                    ),
                    ("Text", "Text/Mart_9780553897852_epub_prl_r1.htm"),
                    ("Cover", "Text/Mart_9780553897852_epub_cvi_r1.htm"),
                ]
            )
        );
    }
}

fn resource(name: &str, media_type: &str, page: bool, size: Option<u64>) -> EpubResource {
    EpubResource {
        file_name: name.to_owned(),
        media_type: media_type.to_owned(),
        sub_type: if page {
            MediaFileSubType::EpubPage
        } else {
            MediaFileSubType::EpubAsset
        },
        file_size: size,
    }
}

#[test]
fn sample_book_lists_spine_pages_then_assets() {
    let path = fixture("The Incomplete Theft - Ralph Burke.epub");
    assert!(is_epub(&path));
    let mut epub = EpubPackage::open(&path).unwrap();
    let split = |n: u8| {
        format!(
            "OEBPS/@public@vhost@g@gutenberg@html@files@65659@65659-h@65659-h-0.htm_split_00{n}.html"
        )
    };
    let xhtml = "application/xhtml+xml";
    assert_eq!(
        epub.resources().unwrap(),
        vec![
            resource("titlepage.xhtml", xhtml, true, Some(818)),
            resource(&split(0), xhtml, true, Some(1869)),
            resource(&split(1), xhtml, true, Some(911)),
            resource(&split(2), xhtml, true, Some(31228)),
            resource("page_styles.css", "text/css", false, Some(58)),
            resource("stylesheet.css", "text/css", false, Some(2799)),
            resource("cover.jpeg", "image/jpeg", false, Some(56756)),
            resource("nav.xhtml", xhtml, false, Some(621)),
        ]
    );
    // Compressed sizes 424, 903, 469 and 10787 bytes.
    assert_eq!(epub.page_count(), 14);
    assert!(!epub.is_fixed_layout());
    assert_eq!(
        epub.toc().unwrap(),
        vec![leaf("The Incomplete Theft", &split(1))]
    );
    assert_eq!(
        epub.landmarks().unwrap(),
        vec![leaf("Cover", "titlepage.xhtml")]
    );
    assert_eq!(
        epub.page_list().unwrap(),
        Vec::new(),
        "No page-list nav and no NCX"
    );

    let epub = EpubPackage::open(&fixture("epub3.epub")).unwrap();
    let pages: Vec<String> = epub
        .resources()
        .unwrap()
        .into_iter()
        .filter(|resource| resource.sub_type == MediaFileSubType::EpubPage)
        .map(|resource| resource.file_name)
        .collect();
    assert_eq!(pages, ["titlepage.xhtml", "page_1.xhtml"]);
}

/// A ZIP of stored members.
pub(super) fn write_zip(dir: &TestDir, members: &[(&str, impl AsRef<[u8]>)]) -> PathBuf {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in members {
        let data = data.as_ref();
        let offset = u32::try_from(archive.len()).unwrap();
        let header = |record: &mut Vec<u8>| {
            for value in [20_u16, 1 << 11, 0, 0, 0] {
                record.extend(value.to_le_bytes());
            }
            let size = u32::try_from(data.len()).unwrap();
            for value in [crc32fast::hash(data), size, size] {
                record.extend(value.to_le_bytes());
            }
            record.extend(u16::try_from(name.len()).unwrap().to_le_bytes());
            record.extend(0_u16.to_le_bytes());
        };
        archive.extend(0x0403_4b50_u32.to_le_bytes());
        header(&mut archive);
        archive.extend(name.as_bytes());
        archive.extend(data);
        directory.extend(0x0201_4b50_u32.to_le_bytes());
        directory.extend(20_u16.to_le_bytes());
        header(&mut directory);
        directory.extend([0; 10]);
        directory.extend(offset.to_le_bytes());
        directory.extend(name.as_bytes());
    }
    let count = u16::try_from(members.len()).unwrap();
    let directory_offset = u32::try_from(archive.len()).unwrap();
    let directory_size = u32::try_from(directory.len()).unwrap();
    archive.extend(directory);
    archive.extend(0x0605_4b50_u32.to_le_bytes());
    archive.extend([0; 4]);
    archive.extend(count.to_le_bytes());
    archive.extend(count.to_le_bytes());
    archive.extend(directory_size.to_le_bytes());
    archive.extend(directory_offset.to_le_bytes());
    archive.extend([0; 2]);
    let path = dir.path().join("book.epub");
    fs::write(&path, archive).unwrap();
    path
}

pub(super) const CONTAINER: &str =
    r#"<container><rootfiles><rootfile full-path="OPS/book.opf"/></rootfiles></container>"#;

#[test]
fn spine_order_is_not_manifest_order() {
    let dir = TestDir::new().unwrap();
    let opf = r#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf">
      <opf:metadata><opf:meta property="rendition:layout"> pre-paginated </opf:meta></opf:metadata>
      <opf:manifest>
        <opf:item id="a" href="a%20page.xhtml" media-type="application/xhtml+xml"/>
        <opf:item id="b" href="../b.xhtml#top" media-type="application/xhtml+xml"/>
        <opf:item id="img" href="img/1.png" media-type="image/png"/>
      </opf:manifest>
      <opf:spine><opf:itemref idref="b"/><opf:itemref idref="missing"/><opf:itemref idref="a"/></opf:spine>
    </opf:package>"#;
    let path = write_zip(
        &dir,
        &[
            ("mimetype", "application/epub+zip\n"),
            ("META-INF/container.xml", CONTAINER),
            ("OPS/book.opf", opf),
            ("OPS/a page.xhtml", "<html/>"),
        ],
    );
    assert!(is_epub(&path));
    let epub = EpubPackage::open(&path).unwrap();
    let xhtml = "application/xhtml+xml";
    assert_eq!(
        epub.resources().unwrap(),
        vec![
            resource("b.xhtml#top", xhtml, true, None),
            resource("OPS/a page.xhtml", xhtml, true, Some(7)),
            resource("OPS/img/1.png", "image/png", false, None),
        ]
    );
    assert!(
        epub.is_fixed_layout(),
        "Prefixed tags and trimmed text, as jsoup selects"
    );
    // The undecoded `a%20page.xhtml` names no member.
    assert_eq!(epub.page_count(), 0);
}

#[test]
fn broken_packages_fail_with_their_cause() {
    let dir = TestDir::new().unwrap();
    let open = |members: &[(&str, &str)]| EpubPackage::open(&write_zip(&dir, members)).err();
    assert!(matches!(
        open(&[("mimetype", "x")]),
        Some(EpubError::MissingRootfile)
    ));
    assert!(matches!(
        open(&[("META-INF/container.xml", "<container/>")]),
        Some(EpubError::MissingRootfile)
    ));
    assert!(matches!(
        open(&[("META-INF/container.xml", CONTAINER)]),
        Some(EpubError::MissingOpf)
    ));
    let malformed = r#"<package><manifest><item id="a" href="100%.xhtml"/></manifest></package>"#;
    let path = write_zip(
        &dir,
        &[
            ("META-INF/container.xml", CONTAINER),
            ("OPS/book.opf", malformed),
        ],
    );
    let epub = EpubPackage::open(&path).unwrap();
    assert!(
        epub.resources().is_err(),
        "URLDecoder rejects a truncated escape"
    );
    assert!(!is_epub(&fixture("../zip/comment.zip")));
}

#[test]
fn missing_or_empty_nav_falls_back_like_kotlin() {
    let dir = TestDir::new().unwrap();
    let opf = r#"<package><manifest>
        <item id="nav" href="nav.xhtml" properties="nav"/>
        <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
      </manifest>
      <guide><reference title="Start" href="a.xhtml"/><reference title="Blank" href=" "/></guide>
    </package>"#;
    let nav = r#"<html><body><nav epub:type="toc"><ol></ol></nav></body></html>"#;
    let ncx = r"<ncx><navMap><navPoint><navLabel><text>One &amp; &nbsp;two</text></navLabel>
        <content src='a.xhtml'/></navPoint></navMap></ncx>";
    let path = write_zip(
        &dir,
        &[
            ("META-INF/container.xml", CONTAINER),
            ("OPS/book.opf", opf),
            ("OPS/nav.xhtml", nav),
            ("OPS/toc.ncx", ncx),
        ],
    );
    let mut epub = EpubPackage::open(&path).unwrap();
    assert_eq!(epub.toc().unwrap(), vec![leaf("One & two", "OPS/a.xhtml")]);
    assert_eq!(
        epub.landmarks().unwrap(),
        vec![
            leaf("Start", "OPS/a.xhtml"),
            entry("Blank", None, Vec::new()),
        ]
    );
}

#[test]
fn cover_follows_kotlin_precedence() {
    let dir = TestDir::new().unwrap();
    let cover_of = |metadata: &str, items: &str| {
        let opf = format!(
            "<package><metadata>{metadata}</metadata><manifest>{items}</manifest></package>"
        );
        let path = write_zip(
            &dir,
            &[
                ("META-INF/container.xml", CONTAINER),
                ("OPS/book.opf", &opf),
                ("OPS/a b.png", "a"),
                ("OPS/b.png", "b"),
                ("OPS/c.png", "c"),
            ],
        );
        EpubPackage::open(&path).unwrap().cover().unwrap()
    };
    let items = r#"<item id="a" href="a%20b.png"/><item id="b" href="b.png"/>
        <item id="cover-image" href="c.png"/>"#;
    let epub3 = r#"<item id="b" href="b.png" properties="x cover-image"/>"#;
    assert_eq!(
        cover_of(
            r#"<meta name="cover" content="a"/>"#,
            &format!("{items}{epub3}")
        ),
        Some(b"b".to_vec())
    );
    assert_eq!(
        cover_of(r#"<meta name=" Cover " content="a"/>"#, items),
        Some(b"a".to_vec()),
        "jsoup trims the attribute and ignores case; the href is URL-decoded"
    );
    assert_eq!(
        cover_of(r#"<meta name="cover" content=" "/>"#, items),
        Some(b"c".to_vec())
    );
    assert_eq!(
        cover_of(r#"<meta name="cover" content="none"/>"#, items),
        Some(b"c".to_vec())
    );
    assert_eq!(
        cover_of("", r#"<item id="cover-image" href="gone.png"/>"#),
        None
    );
    assert_eq!(cover_of("", r#"<item id="a" href="a.png"/>"#), None);

    let mut sample =
        EpubPackage::open(&fixture("The Incomplete Theft - Ralph Burke.epub")).unwrap();
    assert_eq!(
        sample.cover().unwrap().map(|cover| cover.len()),
        Some(56756)
    );
}
