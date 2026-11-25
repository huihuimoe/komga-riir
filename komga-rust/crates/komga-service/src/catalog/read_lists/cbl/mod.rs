//! Kotlin `ReadListProvider`: a ComicRack reading list (`.cbl`) as a read list request, the
//! document read as Jackson's `XmlMapper` reads Kotlin `ReadingList`. The root's `Name` element
//! names the list; every child element of its `Books` element is a book, whose `Series`, `Number`
//! and `Volume` come from attributes or child elements of that name, the last one winning.

use komga_types::{ReadListRequest, ReadListRequestBook};
use quick_xml::escape;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use thiserror::Error;
use tracing::error;

use crate::metadata::providers::comic_info::{ComicInfoError, decode, integer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ComicRackListError {
    #[error("Error while trying to parse ComicRack ReadingList")]
    Malformed,
    #[error("ReadingList has no Name element")]
    NoName,
    #[error("ReadingList does not contain any Book element")]
    NoBooks,
    #[error("Book is missing series or number")]
    IncompleteBook,
}

impl ComicRackListError {
    /// The error code the web client translates.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::Malformed => "ERR_1015",
            Self::NoBooks => "ERR_1029",
            Self::NoName => "ERR_1030",
            Self::IncompleteBook => "ERR_1031",
        }
    }
}

/// `importFromCbl`.
pub(crate) fn import_from_cbl(content: &[u8]) -> Result<ReadListRequest, ComicRackListError> {
    let list = parse(content).map_err(|parse_error| {
        error!(%parse_error, "Error while trying to parse ComicRack ReadingList");
        ComicRackListError::Malformed
    })?;
    let name = list
        .name
        .filter(|name| !name.trim().is_empty())
        .ok_or(ComicRackListError::NoName)?;
    if list.books.is_empty() {
        return Err(ComicRackListError::NoBooks);
    }
    let books = list
        .books
        .into_iter()
        .map(request_book)
        .collect::<Result<_, _>>()?;
    Ok(ReadListRequest { name, books })
}

/// The series as written, and with its volume appended as `computeSeriesFromSeriesAndVolume`
/// does, that one first.
fn request_book(book: BookFields) -> Result<ReadListRequestBook, ComicRackListError> {
    let (Some(series), Some(number)) = (book.series, book.number) else {
        return Err(ComicRackListError::IncompleteBook);
    };
    if series.trim().is_empty() {
        return Err(ComicRackListError::IncompleteBook);
    }
    let with_volume = match book.volume {
        Some(volume) if volume != 1 => format!("{series} ({volume})"),
        _ => series.clone(),
    };
    let candidates = if with_volume == series {
        vec![series]
    } else {
        vec![with_volume, series]
    };
    Ok(ReadListRequestBook {
        series: candidates,
        number: number.trim().to_owned(),
    })
}

#[derive(Debug, Default)]
struct ReadingList {
    name: Option<String>,
    books: Vec<BookFields>,
}

#[derive(Debug, Default)]
struct BookFields {
    series: Option<String>,
    number: Option<String>,
    volume: Option<i32>,
}

impl BookFields {
    fn from_attributes(start: &BytesStart<'_>) -> Result<Self, ComicInfoError> {
        let mut book = Self::default();
        for attribute in start.attributes() {
            let attribute = attribute.map_err(quick_xml::Error::from)?;
            let value = attribute.normalized_value(XmlVersion::Implicit1_0)?;
            book.set(attribute.key.as_ref(), value.into_owned())?;
        }
        Ok(book)
    }

    fn set(&mut self, name: &str, value: String) -> Result<(), ComicInfoError> {
        match name {
            "Series" => self.series = Some(value),
            "Number" => self.number = Some(value),
            "Volume" => integer(&mut self.volume, "Volume", &value)?,
            // Read only to reject the document as Jackson does when it is not an integer.
            "Year" => integer(&mut None, "Year", &value)?,
            _ => {}
        }
        Ok(())
    }
}

/// The child of the root being read.
enum Section {
    /// `Name`, with its text so far.
    Name(String),
    Books,
    Other,
}

/// What is being read inside `Books`: a book, and the child element of it with its text so far.
#[derive(Default)]
struct BookState {
    book: Option<BookFields>,
    field: Option<(String, String)>,
}

fn parse(content: &[u8]) -> Result<ReadingList, ComicInfoError> {
    let text = decode(content)?;
    let mut reader = Reader::from_str(&text);
    let mut list = ReadingList::default();
    let mut depth = 0_usize;
    let mut has_root = false;
    let mut section = Section::Other;
    let mut state = BookState::default();
    loop {
        match reader.read_event()? {
            Event::Start(start) => {
                depth += 1;
                has_root = true;
                match (depth, &section) {
                    (2, _) => section = enter(start.name().as_ref(), &mut list),
                    (3, Section::Books) => state.book = Some(BookFields::from_attributes(&start)?),
                    (4, Section::Books) => {
                        state.field = Some((start.name().as_ref().to_owned(), String::new()));
                    }
                    _ => {}
                }
            }
            Event::Empty(start) => {
                has_root = true;
                match (depth, &section) {
                    (1, _) => {
                        if let Section::Name(name) = enter(start.name().as_ref(), &mut list) {
                            list.name = Some(name);
                        }
                    }
                    (2, Section::Books) => list.books.push(BookFields::from_attributes(&start)?),
                    (3, Section::Books) => {
                        if let Some(book) = &mut state.book {
                            book.set(start.name().as_ref(), String::new())?;
                        }
                    }
                    _ => {}
                }
            }
            Event::End(_) => {
                match (depth, &mut section) {
                    (2, Section::Name(name)) => list.name = Some(std::mem::take(name)),
                    (3, Section::Books) => list.books.extend(state.book.take()),
                    (4, Section::Books) => {
                        if let (Some(book), Some((name, value))) =
                            (&mut state.book, state.field.take())
                        {
                            book.set(&name, value)?;
                        }
                    }
                    _ => {}
                }
                depth = depth.saturating_sub(1);
            }
            Event::Text(text) => push(depth, &mut section, &mut state, &text.xml10_content()),
            Event::CData(text) => push(depth, &mut section, &mut state, &text.xml10_content()),
            Event::GeneralRef(reference) => {
                let raw = format!("&{};", &*reference);
                let resolved = escape::unescape_with(&raw, escape::resolve_xml_entity)?;
                push(depth, &mut section, &mut state, &resolved);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth > 0 {
        Err(ComicInfoError::Unclosed)
    } else if has_root {
        Ok(list)
    } else {
        Err(ComicInfoError::NoRoot)
    }
}

/// Starts reading the child `name` of the root. Another `Books` element replaces the books.
fn enter(name: &str, list: &mut ReadingList) -> Section {
    match name {
        "Name" => Section::Name(String::new()),
        "Books" => {
            list.books.clear();
            Section::Books
        }
        _ => Section::Other,
    }
}

fn push(depth: usize, section: &mut Section, state: &mut BookState, text: &str) {
    match (depth, section) {
        (2, Section::Name(name)) => name.push_str(text),
        (4, Section::Books) => {
            if let Some((_, value)) = &mut state.field {
                value.push_str(text);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
