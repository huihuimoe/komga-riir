//! Kotlin `ComicInfo`: a ComicRack `ComicInfo.xml` document, read as Jackson's `XmlMapper`
//! reads it. Elements of the root element fill the fields of the same name, the last occurrence
//! winning; other elements are ignored. Malformed or truncated XML, or an integer field that does
//! not hold an integer, rejects the whole document.

use quick_xml::Reader;
use quick_xml::escape::{self, EscapeError};
use quick_xml::events::Event;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum ComicInfoError {
    #[error("Malformed XML: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("Malformed XML reference: {0}")]
    Reference(#[from] EscapeError),
    #[error("The document is neither UTF-8 nor UTF-16 text")]
    Encoding,
    #[error("The document has no root element")]
    NoRoot,
    #[error("The document ends inside an element")]
    Unclosed,
    #[error("{element} is not an integer: {value}")]
    Integer {
        element: &'static str,
        value: String,
    },
}

/// The fields Komga reads, as written: strings are neither trimmed nor checked.
#[derive(Debug, Default, PartialEq)]
pub(super) struct ComicInfo {
    pub title: Option<String>,
    pub series: Option<String>,
    pub number: Option<String>,
    pub count: Option<i32>,
    pub volume: Option<i32>,
    pub alternate_series: Option<String>,
    pub alternate_number: Option<String>,
    pub summary: Option<String>,
    pub year: Option<i32>,
    pub month: Option<i32>,
    pub day: Option<i32>,
    pub writer: Option<String>,
    pub penciller: Option<String>,
    pub inker: Option<String>,
    pub colorist: Option<String>,
    pub letterer: Option<String>,
    pub cover_artist: Option<String>,
    pub editor: Option<String>,
    pub translator: Option<String>,
    pub publisher: Option<String>,
    pub genre: Option<String>,
    pub tags: Option<String>,
    pub web: Option<String>,
    pub language_iso: Option<String>,
    pub manga: Option<String>,
    pub characters: Option<String>,
    pub teams: Option<String>,
    pub locations: Option<String>,
    pub story_arc: Option<String>,
    pub story_arc_number: Option<String>,
    pub series_group: Option<String>,
    pub age_rating: Option<String>,
    pub gtin: Option<String>,
}

impl ComicInfo {
    fn set(&mut self, element: &str, value: String) -> Result<(), ComicInfoError> {
        let field = match element {
            "Title" => &mut self.title,
            "Series" => &mut self.series,
            "Number" => &mut self.number,
            "AlternateSeries" => &mut self.alternate_series,
            "AlternateNumber" => &mut self.alternate_number,
            "Summary" => &mut self.summary,
            "Writer" => &mut self.writer,
            "Penciller" => &mut self.penciller,
            "Inker" => &mut self.inker,
            "Colorist" => &mut self.colorist,
            "Letterer" => &mut self.letterer,
            "CoverArtist" => &mut self.cover_artist,
            "Editor" => &mut self.editor,
            "Translator" => &mut self.translator,
            "Publisher" => &mut self.publisher,
            "Genre" => &mut self.genre,
            "Tags" => &mut self.tags,
            "Web" => &mut self.web,
            "LanguageISO" => &mut self.language_iso,
            "Manga" => &mut self.manga,
            "Characters" => &mut self.characters,
            "Teams" => &mut self.teams,
            "Locations" => &mut self.locations,
            "StoryArc" => &mut self.story_arc,
            "StoryArcNumber" => &mut self.story_arc_number,
            "SeriesGroup" => &mut self.series_group,
            "AgeRating" => &mut self.age_rating,
            "GTIN" => &mut self.gtin,
            "Count" => return integer(&mut self.count, "Count", &value),
            "Volume" => return integer(&mut self.volume, "Volume", &value),
            "Year" => return integer(&mut self.year, "Year", &value),
            "Month" => return integer(&mut self.month, "Month", &value),
            "Day" => return integer(&mut self.day, "Day", &value),
            // Read only to reject the document as Jackson does when they are not integers.
            "AlternateCount" => return integer(&mut None, "AlternateCount", &value),
            "PageCount" => return integer(&mut None, "PageCount", &value),
            _ => return Ok(()),
        };
        *field = Some(value);
        Ok(())
    }
}

/// Jackson's `Integer` from text: trimmed, with blank text and `null` as no value.
pub(crate) fn integer(
    field: &mut Option<i32>,
    element: &'static str,
    value: &str,
) -> Result<(), ComicInfoError> {
    let text = value.trim();
    *field = if text.is_empty() || text == "null" {
        None
    } else {
        Some(text.parse().map_err(|_| ComicInfoError::Integer {
            element,
            value: value.to_owned(),
        })?)
    };
    Ok(())
}

pub(super) fn parse(content: &[u8]) -> Result<ComicInfo, ComicInfoError> {
    let text = decode(content)?;
    let mut reader = Reader::from_str(&text);
    let mut comic_info = ComicInfo::default();
    let mut depth = 0_usize;
    let mut has_root = false;
    // The child element of the root being read, and its text so far.
    let mut field: Option<(String, String)> = None;
    loop {
        match reader.read_event()? {
            Event::Start(start) => {
                depth += 1;
                has_root = true;
                if depth == 2 {
                    let name = start.name().as_ref().to_owned();
                    field = Some((name, String::new()));
                }
            }
            Event::Empty(start) => {
                has_root = true;
                if depth == 1 {
                    comic_info.set(start.name().as_ref(), String::new())?;
                }
            }
            Event::End(_) => {
                if depth == 2
                    && let Some((name, value)) = field.take()
                {
                    comic_info.set(&name, value)?;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(text) if depth == 2 => push(&mut field, &text.xml10_content()),
            Event::CData(text) if depth == 2 => push(&mut field, &text.xml10_content()),
            Event::GeneralRef(reference) => {
                let raw = format!("&{};", &*reference);
                let resolved = escape::unescape_with(&raw, escape::resolve_xml_entity)?;
                if depth == 2 {
                    push(&mut field, &resolved);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth > 0 {
        Err(ComicInfoError::Unclosed)
    } else if has_root {
        Ok(comic_info)
    } else {
        Err(ComicInfoError::NoRoot)
    }
}

fn push(field: &mut Option<(String, String)>, text: &str) {
    if let Some((_, value)) = field {
        value.push_str(text);
    }
}

/// UTF-8, with or without a byte order mark, or UTF-16 with one.
pub(crate) fn decode(content: &[u8]) -> Result<String, ComicInfoError> {
    let utf16 = |bytes: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| from(*pair))
            .collect();
        String::from_utf16(&units).map_err(|_| ComicInfoError::Encoding)
    };
    if let Some(rest) = content.strip_prefix(b"\xff\xfe") {
        utf16(rest, u16::from_le_bytes)
    } else if let Some(rest) = content.strip_prefix(b"\xfe\xff") {
        utf16(rest, u16::from_be_bytes)
    } else {
        let content = content.strip_prefix(b"\xef\xbb\xbf").unwrap_or(content);
        String::from_utf8(content.to_vec()).map_err(|_| ComicInfoError::Encoding)
    }
}

#[cfg(test)]
mod tests;
