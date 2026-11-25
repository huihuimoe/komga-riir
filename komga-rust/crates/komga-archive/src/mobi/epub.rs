//! Virtual EPUB view, Readium positions, and dynamic EPUB/KEPUB packaging for [`MobiPublication`].

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::Write;

use flate2::Compression;
use flate2::write::DeflateEncoder;
use jiff::civil::DateTime;
use komga_types::{EpubTocEntry, MediaFileSubType, R2Locator};

use super::html::xml_escape_attr;
use super::{MobiError, MobiPublication};
use crate::epub::{EpubResource, kepub, positions};
use crate::zip::writer::ZipStreamWriter;

const EPUB_MEDIA_TYPE: &str = "application/epub+zip";
const XHTML_MEDIA_TYPE: &str = "application/xhtml+xml";
const CONTAINER_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;

impl MobiPublication {
    /// Virtual EPUB resources: spine chapters as [`MediaFileSubType::EpubPage`], followed by the
    /// navigation document and embedded images as [`MediaFileSubType::EpubAsset`].
    #[must_use]
    pub fn resources(&self) -> Vec<EpubResource> {
        let mut list = Vec::with_capacity(self.chapters.len() + self.images.len() + 1);
        for chapter in &self.chapters {
            list.push(EpubResource {
                file_name: chapter.path.clone(),
                media_type: XHTML_MEDIA_TYPE.to_owned(),
                sub_type: MediaFileSubType::EpubPage,
                file_size: Some(chapter.xhtml.len() as u64),
            });
        }
        let nav = self.nav_xhtml();
        list.push(EpubResource {
            file_name: "OEBPS/nav.xhtml".to_owned(),
            media_type: XHTML_MEDIA_TYPE.to_owned(),
            sub_type: MediaFileSubType::EpubAsset,
            file_size: Some(nav.len() as u64),
        });
        for image in &self.images {
            list.push(EpubResource {
                file_name: image.path.clone(),
                media_type: image.media_type.to_owned(),
                sub_type: MediaFileSubType::EpubAsset,
                file_size: Some(image.bytes.len() as u64),
            });
        }
        list
    }

    /// Virtual EPUB page count matching `EpubPackage::page_count`: one page per started KiB of
    /// each deflated spine chapter.
    #[must_use]
    pub fn page_count(&self) -> u64 {
        self.chapters
            .iter()
            .map(|chapter| {
                deflated_size(chapter.xhtml.as_bytes())
                    .div_ceil(1024)
                    .max(1)
            })
            .sum()
    }

    /// Readium positions with `koboSpan` offsets computed through the shared EPUB position builder.
    #[must_use]
    pub fn positions(&self, resources: &[EpubResource]) -> Vec<R2Locator> {
        let reading_order: Vec<&EpubResource> = resources
            .iter()
            .filter(|resource| resource.sub_type == MediaFileSubType::EpubPage)
            .collect();
        let mut kobo_positions: HashMap<String, Vec<(String, f32)>> = HashMap::new();
        for chapter in &self.chapters {
            let transformed = kepub::transform_content(chapter.xhtml.as_bytes());
            let spans = positions::spans_for_document(&transformed, chapter.xhtml.len() as u64);
            kobo_positions.insert(chapter.path.clone(), spans);
        }
        positions::build_positions(&reading_order, false, &kobo_positions)
    }

    /// Reads one virtual EPUB member by path (leading `/` trimmed).
    #[must_use]
    pub fn resource_bytes(&self, name: &str) -> Option<Vec<u8>> {
        let name = name.trim_start_matches('/');
        match name {
            "" => None,
            "mimetype" => Some(EPUB_MEDIA_TYPE.as_bytes().to_vec()),
            "META-INF/container.xml" => Some(CONTAINER_XML.to_vec()),
            "OEBPS/content.opf" => Some(self.opf_xml().into_bytes()),
            "OEBPS/nav.xhtml" => Some(self.nav_xhtml().into_bytes()),
            _ => self
                .chapters
                .iter()
                .find(|chapter| chapter.path == name)
                .map(|chapter| chapter.xhtml.as_bytes().to_vec())
                .or_else(|| {
                    self.images
                        .iter()
                        .find(|image| image.path == name)
                        .map(|image| image.bytes.clone())
                }),
        }
    }

    /// Streams this virtual publication as a valid EPUB 3 archive into `out`.
    ///
    /// # Errors
    /// Write failure on `out`.
    pub fn write_epub(&self, out: &mut impl Write, modified: DateTime) -> Result<(), MobiError> {
        self.write_package(out, modified, false)
    }

    /// Streams this virtual publication as a KEPUB archive into `out`, reusing `epub::kepub`
    /// content and OPF transforms so `koboSpan` IDs match [`Self::positions`].
    ///
    /// # Errors
    /// Write failure on `out` or OPF transform error.
    pub fn write_kepub(&self, out: &mut impl Write, modified: DateTime) -> Result<(), MobiError> {
        self.write_package(out, modified, true)
    }

    fn write_package(
        &self,
        out: &mut impl Write,
        modified: DateTime,
        as_kepub: bool,
    ) -> Result<(), MobiError> {
        let dummy_titlepage = as_kepub
            && self.chapters.first().is_some_and(|first| {
                kepub::needs_dummy_titlepage(&first.path, first.xhtml.as_bytes())
            });
        let opf = self.opf_xml();
        let opf_bytes = if as_kepub {
            kepub::transform_opf(opf.as_bytes(), "cover-image", dummy_titlepage)
                .map_err(|error| MobiError::Invalid(format!("transform KEPUB OPF: {error}")))?
        } else {
            opf.into_bytes()
        };
        let nav_bytes = self.nav_xhtml().into_bytes();

        let mut zip = ZipStreamWriter::with_compression(modified, Compression::default());
        out.write_all(&zip.stored_entry("mimetype", EPUB_MEDIA_TYPE.as_bytes()))?;
        write_entry(out, &mut zip, "META-INF/container.xml", CONTAINER_XML)?;
        write_entry(out, &mut zip, "OEBPS/content.opf", &opf_bytes)?;
        write_entry(out, &mut zip, "OEBPS/nav.xhtml", &nav_bytes)?;

        for chapter in &self.chapters {
            if as_kepub {
                let transformed = kepub::transform_content(chapter.xhtml.as_bytes());
                write_entry(out, &mut zip, &chapter.path, transformed.as_bytes())?;
            } else {
                write_entry(out, &mut zip, &chapter.path, chapter.xhtml.as_bytes())?;
            }
        }
        for image in &self.images {
            write_entry(out, &mut zip, &image.path, &image.bytes)?;
        }
        if dummy_titlepage {
            let name = format!("OEBPS/{}", kepub::DUMMY_TITLEPAGE_HREF);
            write_entry(out, &mut zip, &name, kepub::DUMMY_TITLEPAGE.as_bytes())?;
        }
        out.write_all(&zip.finish())?;
        Ok(())
    }

    fn opf_xml(&self) -> String {
        let (manifest, spine) = self.opf_manifest_and_spine();
        let metadata = self.opf_metadata_xml();
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">{metadata}</metadata><manifest>{manifest}</manifest><spine>{spine}</spine></package>"#
        )
    }

    fn opf_manifest_and_spine(&self) -> (String, String) {
        let mut manifest = String::from(
            r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#,
        );
        let mut spine = String::new();
        for (index, chapter) in self.chapters.iter().enumerate() {
            let href = chapter.path.strip_prefix("OEBPS/").unwrap_or(&chapter.path);
            let _ = write!(
                manifest,
                r#"<item id="chapter-{index:04}" href="{}" media-type="application/xhtml+xml"/>"#,
                xml_escape_attr(href),
            );
            let _ = write!(spine, r#"<itemref idref="chapter-{index:04}"/>"#);
        }
        for (index, image) in self.images.iter().enumerate() {
            let href = image.path.strip_prefix("OEBPS/").unwrap_or(&image.path);
            let is_cover = self.cover_image_index == Some(index);
            let id = if is_cover {
                "cover-image".to_owned()
            } else {
                format!("asset-{index:04}")
            };
            let properties = if is_cover {
                r#" properties="cover-image""#
            } else {
                ""
            };
            let _ = write!(
                manifest,
                r#"<item id="{id}" href="{}" media-type="{}"{properties}/>"#,
                xml_escape_attr(href),
                xml_escape_attr(image.media_type),
            );
        }
        (manifest, spine)
    }

    fn opf_metadata_xml(&self) -> String {
        let identifier = self
            .metadata
            .identifiers
            .first()
            .cloned()
            .unwrap_or_else(|| format!("urn:komga:mobi:{}", self.unique_id));
        let title = self.metadata.title.as_deref().unwrap_or("Untitled");
        let language = self.metadata.language.as_deref().unwrap_or("en");
        let mut out = format!(
            r#"<dc:identifier id="pub-id">{}</dc:identifier><dc:title>{}</dc:title><dc:language>{}</dc:language>"#,
            xml_escape_attr(&identifier),
            xml_escape_attr(title),
            xml_escape_attr(language),
        );
        for creator in &self.metadata.creators {
            let _ = write!(
                out,
                "<dc:creator>{}</dc:creator>",
                xml_escape_attr(&creator.name)
            );
        }
        if let Some(description) = &self.metadata.description {
            let _ = write!(
                out,
                "<dc:description>{}</dc:description>",
                xml_escape_attr(description)
            );
        }
        if let Some(publisher) = &self.metadata.publisher {
            let _ = write!(
                out,
                "<dc:publisher>{}</dc:publisher>",
                xml_escape_attr(publisher)
            );
        }
        if let Some(date) = &self.metadata.date {
            let _ = write!(out, "<dc:date>{}</dc:date>", xml_escape_attr(date));
        }
        for subject in &self.metadata.subjects {
            let _ = write!(out, "<dc:subject>{}</dc:subject>", xml_escape_attr(subject));
        }
        if self.cover_image_index.is_some() {
            out.push_str(r#"<meta name="cover" content="cover-image"/>"#);
        }
        out
    }

    fn nav_xhtml(&self) -> String {
        let mut list = String::new();
        write_toc_items(&mut list, &self.toc);
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><meta charset="UTF-8"/><title>Navigation</title></head><body><nav epub:type="toc"><ol>{list}</ol></nav></body></html>"#
        )
    }
}

fn write_toc_items(out: &mut String, entries: &[EpubTocEntry]) {
    for entry in entries {
        out.push_str("<li>");
        if let Some(href) = &entry.href {
            let rel = href.strip_prefix("OEBPS/").unwrap_or(href);
            let _ = write!(
                out,
                r#"<a href="{}">{}</a>"#,
                xml_escape_attr(rel),
                xml_escape_attr(&entry.title),
            );
        } else {
            let _ = write!(out, "<span>{}</span>", xml_escape_attr(&entry.title));
        }
        if !entry.children.is_empty() {
            out.push_str("<ol>");
            write_toc_items(out, &entry.children);
            out.push_str("</ol>");
        }
        out.push_str("</li>");
    }
}

fn write_entry(
    out: &mut impl Write,
    zip: &mut ZipStreamWriter,
    name: &str,
    content: &[u8],
) -> Result<(), MobiError> {
    out.write_all(&zip.start_entry(name))?;
    out.write_all(&zip.write(content))?;
    out.write_all(&zip.finish_entry())?;
    Ok(())
}

fn deflated_size(bytes: &[u8]) -> u64 {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    let _ = encoder.write_all(bytes);
    encoder
        .finish()
        .map_or(bytes.len() as u64, |v| v.len() as u64)
}
