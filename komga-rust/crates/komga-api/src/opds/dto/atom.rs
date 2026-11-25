//! Kotlin `opds/v1/dto`: Atom feeds with OPDS links, and the OpenSearch description.

use std::io;

use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesText, Event};

const ATOM: &str = "http://www.w3.org/2005/Atom";
const OPENSEARCH: &str = "http://a9.com/-/spec/opensearch/1.1/";
const PSE: &str = "http://vaemendis.net/opds-pse/ns";

pub(in crate::opds) const NAVIGATION: &str =
    "application/atom+xml;profile=opds-catalog;kind=navigation";
pub(in crate::opds) const ACQUISITION: &str =
    "application/atom+xml;profile=opds-catalog;kind=acquisition";

/// `OpdsFeed`, written by Komga.
pub(in crate::opds) struct Feed {
    pub id: String,
    pub title: String,
    pub updated: String,
    pub links: Vec<Link>,
    pub entries: Vec<Entry>,
}

/// `OpdsEntryNavigation` with a single link and no authors, else `OpdsEntryAcquisition`.
pub(in crate::opds) struct Entry {
    pub title: String,
    pub updated: String,
    pub id: String,
    pub content: String,
    pub authors: Vec<String>,
    pub links: Vec<Link>,
}

/// `OpdsLink`; `stream` makes it `OpdsLinkPageStreaming`.
pub(in crate::opds) struct Link {
    pub media_type: String,
    pub rel: &'static str,
    pub href: String,
    pub stream: Option<PageStream>,
}

/// The `pse` attributes of a page streaming link.
pub(in crate::opds) struct PageStream {
    pub count: i32,
    pub last_read: Option<i32>,
    pub last_read_date: Option<String>,
}

impl Link {
    pub(in crate::opds) fn new(media_type: &str, rel: &'static str, href: String) -> Self {
        Self {
            media_type: media_type.to_owned(),
            rel,
            href,
            stream: None,
        }
    }
}

impl Feed {
    pub(in crate::opds) fn to_xml(&self) -> Vec<u8> {
        document(|writer| {
            writer
                .create_element("feed")
                .with_attributes([("xmlns", ATOM), ("xmlns:pse", PSE)])
                .write_inner_content(|writer| {
                    text(writer, "id", &self.id)?;
                    text(writer, "title", &self.title)?;
                    text(writer, "updated", &self.updated)?;
                    writer
                        .create_element("author")
                        .write_inner_content(|writer| {
                            text(writer, "name", "Komga")?;
                            text(writer, "uri", "https://github.com/huihuimoe/komga-riir")
                        })?;
                    for link in &self.links {
                        link.write(writer)?;
                    }
                    for entry in &self.entries {
                        entry.write(writer)?;
                    }
                    Ok(())
                })?;
            Ok(())
        })
    }
}

impl Entry {
    fn write(&self, writer: &mut Writer<Vec<u8>>) -> io::Result<()> {
        writer
            .create_element("entry")
            .write_inner_content(|writer| {
                text(writer, "title", &self.title)?;
                text(writer, "updated", &self.updated)?;
                text(writer, "id", &self.id)?;
                text(writer, "content", &self.content.replace('\n', "<br/>"))?;
                for author in &self.authors {
                    writer
                        .create_element("author")
                        .write_inner_content(|writer| text(writer, "name", author))?;
                }
                for link in &self.links {
                    link.write(writer)?;
                }
                Ok(())
            })?;
        Ok(())
    }
}

impl Link {
    fn write(&self, writer: &mut Writer<Vec<u8>>) -> io::Result<()> {
        let mut element = writer.create_element("link").with_attributes([
            ("type", self.media_type.as_str()),
            ("rel", self.rel),
            ("href", self.href.as_str()),
        ]);
        if let Some(stream) = &self.stream {
            element = element.with_attribute(("pse:count", stream.count.to_string().as_str()));
            if let Some(page) = stream.last_read {
                element = element.with_attribute(("pse:lastRead", page.to_string().as_str()));
            }
            if let Some(date) = &stream.last_read_date {
                element = element.with_attribute(("pse:lastReadDate", date.as_str()));
            }
        }
        element.write_empty()?;
        Ok(())
    }
}

/// `OpenSearchDescription` of the series search.
pub(in crate::opds) fn open_search(template: &str) -> Vec<u8> {
    document(|writer| {
        writer
            .create_element("OpenSearchDescription")
            .with_attribute(("xmlns", OPENSEARCH))
            .write_inner_content(|writer| {
                text(writer, "ShortName", "Search")?;
                text(writer, "Description", "Search for series")?;
                text(writer, "InputEncoding", "UTF-8")?;
                text(writer, "OutputEncoding", "UTF-8")?;
                writer
                    .create_element("Url")
                    .with_attributes([("template", template), ("type", ACQUISITION)])
                    .write_empty()?;
                Ok(())
            })?;
        Ok(())
    })
}

fn document(body: impl FnOnce(&mut Writer<Vec<u8>>) -> io::Result<()>) -> Vec<u8> {
    let mut writer = Writer::new(Vec::new());
    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .and_then(|()| body(&mut writer))
        .expect("Writing to memory cannot fail");
    writer.into_inner()
}

fn text(writer: &mut Writer<Vec<u8>>, name: &str, value: &str) -> io::Result<()> {
    writer
        .create_element(name)
        .write_text_content(BytesText::new(value))?;
    Ok(())
}
