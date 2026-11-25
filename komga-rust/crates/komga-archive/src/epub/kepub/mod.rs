//! kepubify v4.0.4 `Convert` with its default options: an EPUB repackaged as a KEPUB, with
//! `koboSpan` content documents, a Kobo-friendly package document and, where the first page is
//! not a title page, a blank one before it.

mod content;
mod dom;
mod kobo_span;
mod render;
mod sentences;

use std::io::{Read, Write};

use flate2::Compression;
use jiff::civil::DateTime;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

pub(crate) use self::content::transform_content;
pub(super) use self::kobo_span::{KoboSpan, has_kobo_span, kobo_spans};
use super::{EpubError, EpubPackage, ManifestItem, href};
use crate::zip::ZipError;
use crate::zip::writer::ZipStreamWriter;

const DUMMY_TITLEPAGE_ID: &str = "kepubify-titlepage-dummy";
pub(crate) const DUMMY_TITLEPAGE_HREF: &str = "kepubify-titlepage-dummy.xhtml";
pub(crate) const DUMMY_TITLEPAGE: &str = r#"<!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml" lang="en"><head><title></title></head><body><p style="text-align: center; margin: 4em 0; font-size: .7em; font-style: italic;">Page intentionally left blank by kepubify.</p></body></html>"#;

/// kepubify `TransformFileFilter`: reader and OS leftovers that do not belong in the book.
fn is_filtered(name: &str) -> bool {
    let base = name.rsplit('/').next().unwrap_or(name);
    // kepubify only drops direct children of `__MACOSX`; the whole tree is leftovers.
    name.starts_with("__MACOSX/")
        || matches!(
            base,
            "calibre_bookmarks.txt"
                | "iTunesMetadata.plist"
                | "iTunesArtwork.plist"
                | ".DS_STORE"
                | "thumbs.db"
                | "Thumbs.db"
        )
}

/// kepubify `epubContentDocuments`: XHTML or HTML by media type or extension.
fn is_content_document(item: &ManifestItem) -> bool {
    matches!(
        item.media_type.as_str(),
        "application/xhtml+xml" | "text/html"
    ) || has_html_extension(&item.href.to_lowercase())
}

fn has_html_extension(href: &str) -> bool {
    [".htm", ".html", ".xhtml"]
        .iter()
        .any(|extension| href.ends_with(extension))
}

fn is_dummy_titlepage_candidate(href: &str) -> bool {
    let base = href.rsplit('/').next().unwrap_or_default().to_lowercase();
    has_html_extension(&base) && !base.contains("cover") && !base.contains("title")
}

/// Whether a first linear page with `href` and `content` needs kepubify's blank dummy title page.
pub(crate) fn needs_dummy_titlepage(href: &str, content: &[u8]) -> bool {
    is_dummy_titlepage_candidate(href) && content::needs_dummy_titlepage(content)
}

impl EpubPackage {
    /// Writes the KEPUB form of the book to `out`: the stored `mimetype` first, then the members
    /// in archive order, compressed.
    ///
    /// # Errors
    /// An unreadable member or href, a package document that cannot be rewritten, or a failed
    /// write.
    pub fn write_kepub(
        &mut self,
        out: &mut impl Write,
        modified: DateTime,
    ) -> Result<(), EpubError> {
        let content_documents = self.content_documents()?;
        let dummy_titlepage = self.needs_dummy_titlepage()?;
        let mut opf = self.kepub_opf(dummy_titlepage)?;
        let mut zip = ZipStreamWriter::with_compression(modified, Compression::default());
        let write =
            |out: &mut dyn Write, bytes: Vec<u8>| out.write_all(&bytes).map_err(ZipError::Io);
        write(out, zip.stored_entry("mimetype", b"application/epub+zip"))?;
        for index in 0..self.archive.entries().len() {
            let name = self.archive.entries()[index].name().to_owned();
            if name == "mimetype" || name.ends_with('/') || is_filtered(&name) {
                continue;
            }
            let mut content = Vec::new();
            self.archive
                .entry_reader(index)?
                .read_to_end(&mut content)
                .map_err(ZipError::Io)?;
            let content = if name == self.opf_path {
                std::mem::take(&mut opf)
            } else if content_documents.contains(&name.to_lowercase()) {
                content::transform_content(&content).into_bytes()
            } else {
                content
            };
            write(out, zip.start_entry(&name))?;
            write(out, zip.write(&content))?;
            write(out, zip.finish_entry())?;
        }
        if dummy_titlepage {
            let name = href::normalize_href(self.opf_dir(), DUMMY_TITLEPAGE_HREF);
            write(out, zip.start_entry(&name))?;
            write(out, zip.write(DUMMY_TITLEPAGE.as_bytes()))?;
            write(out, zip.finish_entry())?;
        }
        write(out, zip.finish())?;
        Ok(())
    }

    /// Lowercase member names of the content documents; kepubify falls back to a
    /// case-insensitive match when no member has the exact name.
    fn content_documents(&self) -> Result<Vec<String>, EpubError> {
        self.manifest
            .iter()
            .filter(|item| is_content_document(item))
            .map(|item| {
                let name = href::normalize_href(self.opf_dir(), &href::url_decode(&item.href)?);
                Ok(name.to_lowercase())
            })
            .collect()
    }

    /// kepubify `transformDummyTitlepageRequired`: only an HTML first linear page whose name
    /// does not already say it is a cover or title page is looked at.
    fn needs_dummy_titlepage(&mut self) -> Result<bool, EpubError> {
        let first = self
            .opf
            .select_child_of(
                |parent| parent.is_any_ns("spine"),
                |child| child.is_any_ns("itemref"),
            )
            .into_iter()
            .find(|itemref| itemref.attr("linear") != "no")
            .and_then(|itemref| self.manifest_item(itemref.attr("idref")));
        let Some(item) = first else {
            return Ok(false);
        };
        if !is_dummy_titlepage_candidate(&item.href) {
            return Ok(false);
        }
        let name = href::normalize_href(self.opf_dir(), &href::url_decode(&item.href)?);
        Ok(self
            .archive
            .read_named(&name)?
            .is_some_and(|content| content::needs_dummy_titlepage(&content)))
    }

    /// kepubify `TransformOPF`, with the dummy title page added first in the spine. The rest of
    /// the document keeps its formatting.
    fn kepub_opf(&mut self, dummy_titlepage: bool) -> Result<Vec<u8>, EpubError> {
        let cover_id = self
            .opf
            .descendants()
            .into_iter()
            .find(|(_, element)| element.is_any_ns("meta") && element.attr("name") == "cover")
            .map_or("cover", |(_, meta)| meta.attr("content"))
            .to_owned();
        let source = self
            .archive
            .read_named(&self.opf_path)?
            .ok_or(EpubError::MissingOpf)?;
        transform_opf(&source, &cover_id, dummy_titlepage)
            .map_err(|error| EpubError::MalformedOpf(error.to_string()))
    }
}

/// Elements kepubify removes: Calibre's timestamp and the book producer.
fn is_removed(element: &BytesStart<'_>) -> bool {
    let has = |key: &str, value: &str| {
        element
            .attributes()
            .flatten()
            .any(|attribute| attribute.key.local_name().as_ref() == key && attribute.value == value)
    };
    match element.local_name().as_ref() {
        "meta" => has("name", "calibre:timestamp"),
        "contributor" => has("role", "bkp"),
        _ => false,
    }
}

fn has_id(element: &BytesStart<'_>, id: &str) -> bool {
    element
        .try_get_attribute("id")
        .ok()
        .flatten()
        .is_some_and(|attribute| attribute.value == id)
}

/// `element` with `properties="cover-image"` in place of any other properties.
fn cover_image(element: &BytesStart<'_>) -> BytesStart<'static> {
    let mut cover = BytesStart::new(element.name().as_ref().to_owned());
    cover.extend_attributes(
        element
            .attributes()
            .flatten()
            .filter(|attribute| attribute.key.as_ref() != "properties"),
    );
    cover.push_attribute(("properties", "cover-image"));
    cover
}

/// A new element named like `sibling`, so that it shares its namespace prefix.
fn sibling_element<'a>(sibling: &str, local: &str, attributes: &[(&'a str, &'a str)]) -> Event<'a> {
    let prefix = sibling.find(':').map_or("", |colon| &sibling[..=colon]);
    let mut element = BytesStart::new(format!("{prefix}{local}"));
    element.extend_attributes(attributes.iter().copied());
    Event::Empty(element)
}

pub(crate) fn transform_opf(
    source: &[u8],
    cover_id: &str,
    dummy_titlepage: bool,
) -> Result<Vec<u8>, quick_xml::Error> {
    let mut reader = Reader::from_reader(source);
    let mut writer = Writer::new(Vec::with_capacity(source.len()));
    let mut skipped_depth = 0_usize;
    loop {
        let event = reader.read_event()?;
        if skipped_depth > 0 {
            match event {
                Event::Start(_) => skipped_depth += 1,
                Event::End(_) => skipped_depth -= 1,
                Event::Eof => break,
                _ => {}
            }
            continue;
        }
        match event {
            Event::Eof => break,
            Event::Start(element) if is_removed(&element) => skipped_depth = 1,
            Event::Empty(element) if is_removed(&element) => {}
            Event::Start(element) if has_id(&element, cover_id) => {
                writer.write_event(Event::Start(cover_image(&element)))?;
            }
            Event::Empty(element) if has_id(&element, cover_id) => {
                writer.write_event(Event::Empty(cover_image(&element)))?;
            }
            Event::Start(element)
                if dummy_titlepage && element.local_name().as_ref() == "spine" =>
            {
                let itemref = sibling_element(
                    element.name().as_ref(),
                    "itemref",
                    &[("idref", DUMMY_TITLEPAGE_ID)],
                );
                writer.write_event(Event::Start(element))?;
                writer.write_event(itemref)?;
            }
            Event::End(element)
                if dummy_titlepage && element.local_name().as_ref() == "manifest" =>
            {
                let item = sibling_element(
                    element.name().as_ref(),
                    "item",
                    &[
                        ("id", DUMMY_TITLEPAGE_ID),
                        ("href", DUMMY_TITLEPAGE_HREF),
                        ("media-type", "application/xhtml+xml"),
                    ],
                );
                writer.write_event(item)?;
                writer.write_event(Event::End(element))?;
            }
            event => writer.write_event(event)?,
        }
    }
    Ok(writer.into_inner())
}

#[cfg(test)]
mod tests;
