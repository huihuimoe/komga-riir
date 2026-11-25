use std::io::Cursor;
use std::path::Path;

use jiff::civil::date;
use komga_test_support::TestDir;
use komga_types::MediaFileSubType;

use super::{MobiError, MobiPublication, MobiUnsupportedReason, read_cover, read_metadata};
use crate::epub::EpubPackage;
use crate::zip::ZipArchive;

const TINY_JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xff, 0xd9,
];
const TINY_PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0x00, 0x00,
];

struct MobiBuilder {
    full_name: String,
    compression: u16,
    encryption: u16,
    file_version: u32,
    encoding: u32,
    text_records: Vec<Vec<u8>>,
    extra_records: Vec<Vec<u8>>,
    exth: Vec<(u32, Vec<u8>)>,
    first_image_index: Option<u32>,
    huff_offset_count: Option<(u32, u32)>,
}

impl Default for MobiBuilder {
    fn default() -> Self {
        Self {
            full_name: "Test Full Title".to_owned(),
            compression: 1,
            encryption: 0,
            file_version: 6,
            encoding: 65001,
            text_records: Vec::new(),
            extra_records: Vec::new(),
            exth: Vec::new(),
            first_image_index: None,
            huff_offset_count: None,
        }
    }
}

impl MobiBuilder {
    fn add_exth_str(&mut self, id: u32, value: &str) {
        self.exth.push((id, value.as_bytes().to_vec()));
    }

    fn add_exth_u32(&mut self, id: u32, value: u32) {
        self.exth.push((id, value.to_be_bytes().to_vec()));
    }

    fn build(self) -> Vec<u8> {
        let r0 = self.build_record_zero();
        let mut records =
            Vec::with_capacity(1 + self.text_records.len() + self.extra_records.len());
        records.push(r0);
        records.extend(self.text_records);
        records.extend(self.extra_records);
        pack_pdb("TestBook", records)
    }

    fn build_record_zero(&self) -> Vec<u8> {
        let mut r0 = vec![0_u8; 16 + 232];
        let text_len: u32 = self.text_records.iter().map(|r| r.len() as u32).sum();
        r0[0..2].copy_from_slice(&self.compression.to_be_bytes());
        r0[4..8].copy_from_slice(&text_len.to_be_bytes());
        r0[8..10].copy_from_slice(&(self.text_records.len() as u16).to_be_bytes());
        r0[10..12].copy_from_slice(&4096_u16.to_be_bytes());
        r0[12..14].copy_from_slice(&self.encryption.to_be_bytes());

        let m = 16;
        r0[m..m + 4].copy_from_slice(b"MOBI");
        r0[m + 4..m + 8].copy_from_slice(&232_u32.to_be_bytes());
        r0[m + 8..m + 12].copy_from_slice(&2_u32.to_be_bytes());
        r0[m + 12..m + 16].copy_from_slice(&self.encoding.to_be_bytes());
        r0[m + 16..m + 20].copy_from_slice(&42_u32.to_be_bytes());
        r0[m + 20..m + 24].copy_from_slice(&self.file_version.to_be_bytes());
        r0[m + 76..m + 80].copy_from_slice(&0x09_u32.to_be_bytes());
        r0[m + 92..m + 96]
            .copy_from_slice(&self.first_image_index.unwrap_or(u32::MAX).to_be_bytes());
        if let Some((off, cnt)) = self.huff_offset_count {
            r0[m + 96..m + 100].copy_from_slice(&off.to_be_bytes());
            r0[m + 100..m + 104].copy_from_slice(&cnt.to_be_bytes());
        }
        let exth_flags: u32 = if self.exth.is_empty() { 0 } else { 0x40 };
        r0[m + 112..m + 116].copy_from_slice(&exth_flags.to_be_bytes());
        r0[m + 152..m + 156].copy_from_slice(&u32::MAX.to_be_bytes());
        r0[m + 228..m + 232].copy_from_slice(&u32::MAX.to_be_bytes());

        if !self.exth.is_empty() {
            r0.extend_from_slice(&encode_exth(&self.exth));
        }
        let full_name_offset = r0.len() as u32;
        r0.extend_from_slice(self.full_name.as_bytes());
        r0[m + 68..m + 72].copy_from_slice(&full_name_offset.to_be_bytes());
        r0[m + 72..m + 76].copy_from_slice(&(self.full_name.len() as u32).to_be_bytes());
        r0
    }
}

fn encode_exth(entries: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut exth = Vec::new();
    exth.extend_from_slice(b"EXTH");
    exth.extend_from_slice(&0_u32.to_be_bytes());
    exth.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (rec_type, payload) in entries {
        exth.extend_from_slice(&rec_type.to_be_bytes());
        exth.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
        exth.extend_from_slice(payload);
    }
    while exth.len() % 4 != 0 {
        exth.push(0);
    }
    let total_len = exth.len() as u32;
    exth[4..8].copy_from_slice(&total_len.to_be_bytes());
    exth
}

fn pack_pdb(name: &str, records: Vec<Vec<u8>>) -> Vec<u8> {
    let mut pdb = vec![0_u8; 78 + records.len() * 8];
    let name_bytes = name.as_bytes();
    let copy_len = name_bytes.len().min(31);
    pdb[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
    pdb[60..68].copy_from_slice(b"BOOKMOBI");
    pdb[76..78].copy_from_slice(&(records.len() as u16).to_be_bytes());
    let mut offset = pdb.len();
    for (index, record) in records.iter().enumerate() {
        pdb[78 + index * 8..82 + index * 8].copy_from_slice(&(offset as u32).to_be_bytes());
        offset += record.len();
    }
    for record in records {
        pdb.extend_from_slice(&record);
    }
    pdb
}

fn palmdoc_compress_literal(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for chunk in input.chunks(8) {
        out.push(chunk.len() as u8);
        out.extend_from_slice(chunk);
    }
    out
}

fn rich_palmdoc_mobi() -> Vec<u8> {
    let probe = concat!(
        "<html><head><guide><reference type=\"toc\" filepos=\"0000000090\"/></guide></head><body>",
        "<p><a filepos=\"0000000210\">Chapter One</a></p>",
        "<mbp:pagebreak/>",
        "<p>Hello &amp; <b>world</b><img recindex=\"00002\"></p>",
        "</body></html>",
    );
    let toc_offset = probe.find("<p><a").expect("toc");
    let ch1_offset = probe.find("<p>Hello").expect("ch1");
    let html = format!(
        "<html><head><guide><reference type=\"toc\" filepos=\"{toc_offset:010}\"/></guide></head><body>\
         <p><a filepos=\"{ch1_offset:010}\">Chapter One</a></p>\
         <mbp:pagebreak/>\
         <p>Hello &amp; <b>world</b><img recindex=\"00002\"></p>\
         </body></html>"
    );
    let mut builder = MobiBuilder {
        compression: 2,
        text_records: vec![palmdoc_compress_literal(html.as_bytes())],
        first_image_index: Some(2),
        extra_records: vec![TINY_JPEG.to_vec(), TINY_PNG.to_vec()],
        ..MobiBuilder::default()
    };
    builder.add_exth_str(503, "Updated Title");
    builder.add_exth_str(100, "Alice Author");
    builder.add_exth_str(101, "Example Press");
    builder.add_exth_str(103, "<p>A <b>great</b> book</p>");
    builder.add_exth_str(104, "9780306406157");
    builder.add_exth_str(105, "Fiction");
    builder.add_exth_str(106, "2024-03-15");
    builder.add_exth_str(524, "en-US");
    builder.add_exth_u32(201, 0);
    builder.build()
}

/// Builds HUFF and CDIC records testing short terminal codes, 9-bit codes, and recursive phrases.
fn build_huff_cdic_fixture() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut huff = vec![0_u8; 24 + 256 * 4 + 32 * 8];
    huff[0..8].copy_from_slice(b"HUFF\x00\x00\x00\x18");
    huff[8..12].copy_from_slice(&24_u32.to_be_bytes());
    huff[12..16].copy_from_slice(&(24_u32 + 256 * 4).to_be_bytes());
    for byte in 0..256_u32 {
        let entry: u32 = if byte >= 0xd0 {
            (15 << 8) | 0x80 | 4
        } else {
            9
        };
        let pos = 24 + (byte as usize) * 4;
        huff[pos..pos + 4].copy_from_slice(&entry.to_be_bytes());
    }
    let base9 = 24 + 256 * 4 + 8 * 8;
    huff[base9..base9 + 4].copy_from_slice(&1_u32.to_be_bytes());
    huff[base9 + 4..base9 + 8].copy_from_slice(&4_u32.to_be_bytes());

    let nested = [0xfe_u8];
    let entries: [(&[u8], bool); 4] = [
        (b"<p>Hello ", true),
        (b"World</p>", true),
        (&nested, false),
        (b"!", true),
    ];
    let mut cdic = Vec::new();
    cdic.extend_from_slice(b"CDIC\x00\x00\x00\x10");
    cdic.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    cdic.extend_from_slice(&4_u32.to_be_bytes());
    let (mut offsets, mut payload) = (Vec::new(), Vec::new());
    for (slice, terminal) in entries {
        offsets.push((entries.len() * 2 + payload.len()) as u16);
        let word = (slice.len() as u16) | if terminal { 0x8000 } else { 0 };
        payload.extend_from_slice(&word.to_be_bytes());
        payload.extend_from_slice(slice);
    }
    for off in offsets {
        cdic.extend_from_slice(&off.to_be_bytes());
    }
    cdic.extend_from_slice(&payload);
    (huff, cdic, vec![0b1101_0000, 0b0000_1000])
}

#[test]
fn parses_palmdoc_metadata_cover_rewritten_links_and_guide_toc() {
    let dir = TestDir::new().expect("temp dir");
    let path = dir.path().join("book.mobi");
    std::fs::write(&path, rich_palmdoc_mobi()).expect("write mobi");

    let metadata = read_metadata(&path).expect("metadata");
    assert_eq!(metadata.title.as_deref(), Some("Updated Title"));
    assert_eq!(metadata.description.as_deref(), Some("A great book"));
    assert_eq!(metadata.publisher.as_deref(), Some("Example Press"));
    assert_eq!(metadata.date.as_deref(), Some("2024-03-15"));
    assert_eq!(metadata.language.as_deref(), Some("en-US"));
    assert_eq!(metadata.subjects, ["Fiction"]);
    assert_eq!(metadata.identifiers, ["9780306406157"]);
    assert_eq!(metadata.creators[0].name, "Alice Author");
    assert_eq!(
        read_cover(&path).expect("cover").as_deref(),
        Some(TINY_JPEG)
    );

    let pub_ = MobiPublication::open(&path).expect("open mobi");
    let resources = pub_.resources();
    let pages: Vec<_> = resources
        .iter()
        .filter(|r| r.sub_type == MediaFileSubType::EpubPage)
        .collect();
    assert_eq!(pages.len(), 3);
    assert_eq!(pages[0].file_name, "OEBPS/text/cover.xhtml");
    assert!(pub_.page_count() >= 3);

    let ch0 = String::from_utf8(
        pub_.resource_bytes("OEBPS/text/chapter-0000.xhtml")
            .expect("ch0"),
    )
    .expect("utf8");
    assert!(
        ch0.contains(r#"href="chapter-0001.xhtml""#) && !ch0.contains("filepos="),
        "{ch0}"
    );
    let ch1 = String::from_utf8(
        pub_.resource_bytes("OEBPS/text/chapter-0001.xhtml")
            .expect("ch1"),
    )
    .expect("utf8");
    assert!(
        ch1.contains(r#"src="../images/asset-0002.png""#) && !ch1.contains("recindex="),
        "{ch1}"
    );

    assert_eq!(pub_.toc().len(), 1);
    assert_eq!(pub_.toc()[0].title, "Chapter One");
    assert_eq!(
        pub_.toc()[0].href.as_deref(),
        Some("OEBPS/text/chapter-0001.xhtml")
    );
    assert_eq!(
        pub_.positions(&resources)[0].kobo_span.as_deref(),
        Some("kobo.1.1")
    );
}

#[test]
fn writes_valid_epub_and_kepub_packages() {
    let dir = TestDir::new().expect("temp dir");
    let pub_ = MobiPublication::from_bytes(&rich_palmdoc_mobi()).expect("parse mobi");
    let modified = date(2025, 1, 2).at(3, 4, 5, 0);

    let mut epub_bytes = Vec::new();
    pub_.write_epub(&mut epub_bytes, modified)
        .expect("write_epub");
    let epub_path = dir.path().join("out.epub");
    std::fs::write(&epub_path, &epub_bytes).expect("write out.epub");
    let mut epub = EpubPackage::open(&epub_path).expect("open generated epub");
    assert_eq!(epub.metadata().title.as_deref(), Some("Updated Title"));
    assert_eq!(
        epub.cover().expect("epub cover").as_deref(),
        Some(TINY_JPEG)
    );

    let mut kepub_bytes = Vec::new();
    pub_.write_kepub(&mut kepub_bytes, modified)
        .expect("write_kepub");
    let kepub_path = dir.path().join("out.kepub.epub");
    std::fs::write(&kepub_path, &kepub_bytes).expect("write out.kepub.epub");
    let mut kepub = EpubPackage::open(&kepub_path).expect("open generated kepub");
    let kepub_resources = kepub.resources().expect("kepub resources");
    assert!(kepub.is_kepub(&kepub_resources));
    let mut zip = ZipArchive::new(Cursor::new(kepub_bytes)).expect("valid zip");
    assert_eq!(
        zip.read_named("mimetype").expect("mimetype").as_deref(),
        Some(&b"application/epub+zip"[..])
    );
}

#[test]
fn decompresses_huff_cdic_and_handles_cp1252() {
    let (huff, cdic, compressed) = build_huff_cdic_fixture();
    let builder = MobiBuilder {
        compression: 17480,
        text_records: vec![compressed],
        extra_records: vec![huff, cdic],
        huff_offset_count: Some((2, 2)),
        ..MobiBuilder::default()
    };
    let pub_ = MobiPublication::from_bytes(&builder.build()).expect("huff/cdic");
    let ch = String::from_utf8(
        pub_.resource_bytes("OEBPS/text/chapter-0000.xhtml")
            .expect("ch0"),
    )
    .expect("utf8");
    assert!(ch.contains("Hello World") && ch.contains('!'), "{ch}");

    let cp_builder = MobiBuilder {
        compression: 1,
        encoding: 1252,
        text_records: vec![b"<html><body><p>\x93Hi\x94\x97ok</p></body></html>".to_vec()],
        ..MobiBuilder::default()
    };
    let cp_pub = MobiPublication::from_bytes(&cp_builder.build()).expect("cp1252");
    let cp_ch = String::from_utf8(
        cp_pub
            .resource_bytes("OEBPS/text/chapter-0000.xhtml")
            .expect("ch0"),
    )
    .expect("utf8");
    assert!(cp_ch.contains("\u{201c}Hi\u{201d}\u{2014}ok"), "{cp_ch}");
}

#[test]
fn dual_mobi_reads_mobi6_section_and_stops_images_at_kf8_boundary() {
    let mut builder = MobiBuilder {
        compression: 1,
        text_records: vec![
            b"<html><body><p>Dual <img recindex=\"00001\"/></p></body></html>".to_vec(),
        ],
        first_image_index: Some(2),
        extra_records: vec![TINY_JPEG.to_vec(), b"BOUNDARY".to_vec(), TINY_PNG.to_vec()],
        ..MobiBuilder::default()
    };
    builder.add_exth_u32(121, 3);
    let pub_ = MobiPublication::from_bytes(&builder.build()).expect("dual-mobi");
    assert_eq!(pub_.images.len(), 1);
    assert_eq!(pub_.images[0].media_type, "image/jpeg");
}

#[test]
fn rejects_drm_kf8_unknown_compression_and_corrupt_files() {
    let drm = MobiBuilder {
        encryption: 2,
        text_records: vec![b"<p>x</p>".to_vec()],
        ..MobiBuilder::default()
    };
    assert!(matches!(
        MobiPublication::from_bytes(&drm.build()),
        Err(MobiError::Unsupported(MobiUnsupportedReason::Drm))
    ));

    let kf8 = MobiBuilder {
        file_version: 8,
        text_records: vec![b"<p>x</p>".to_vec()],
        ..MobiBuilder::default()
    };
    assert!(matches!(
        MobiPublication::from_bytes(&kf8.build()),
        Err(MobiError::Unsupported(MobiUnsupportedReason::Kf8))
    ));

    let bad = MobiBuilder {
        compression: 99,
        text_records: vec![b"<p>x</p>".to_vec()],
        ..MobiBuilder::default()
    };
    assert!(matches!(
        MobiPublication::from_bytes(&bad.build()),
        Err(MobiError::Unsupported(MobiUnsupportedReason::Compression(
            99
        )))
    ));

    assert!(matches!(
        MobiPublication::from_bytes(b"not a mobi"),
        Err(MobiError::Invalid(_))
    ));
}

#[test]
fn parses_epub3_mobi_sample() {
    let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mobi/epub3.mobi");
    let pub_ = MobiPublication::open(&sample).expect("open sample epub3.mobi");
    assert_eq!(pub_.metadata().title.as_deref(), Some("zip"));
    assert_eq!(pub_.metadata().language.as_deref(), Some("en"));
    assert!(read_cover(&sample).expect("cover").is_some());
    assert_eq!(pub_.toc()[0].title, "Page 1");
    assert!(!pub_.resources().is_empty());
}
