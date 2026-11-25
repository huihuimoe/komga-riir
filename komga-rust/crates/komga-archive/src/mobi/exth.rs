//! EXTH header parsing, MOBI metadata extraction, and Windows LCID language mapping.

use html5gum::{DefaultEmitter, Token, Tokenizer};

use super::MobiError;
use super::binary::{NO_RECORD, TextEncoding, read_u32, trim_nul};
use crate::epub::{EpubCreator, EpubMetadata};

const MAX_EXTH_RECORDS: usize = 4_096;

#[derive(Default)]
pub(super) struct ExthData {
    pub(super) updated_title: Option<String>,
    pub(super) authors: Vec<String>,
    pub(super) contributors: Vec<String>,
    pub(super) publisher: Option<String>,
    pub(super) description: Option<String>,
    pub(super) isbns: Vec<String>,
    pub(super) subjects: Vec<String>,
    pub(super) date: Option<String>,
    pub(super) language: Option<String>,
    pub(super) cover_offset: Option<usize>,
    pub(super) thumb_offset: Option<usize>,
    pub(super) kf8_boundary: Option<usize>,
}

pub(super) fn parse(slice: &[u8], encoding: TextEncoding) -> Result<ExthData, MobiError> {
    if slice.len() < 12 || slice.get(0..4) != Some(b"EXTH") {
        return Ok(ExthData::default());
    }
    let total_len = (read_u32(slice, 4, "EXTH length")? as usize).min(slice.len());
    let count = read_u32(slice, 8, "EXTH count")? as usize;
    if count > MAX_EXTH_RECORDS {
        return Err(MobiError::LimitExceeded("too many EXTH records"));
    }
    let mut data = ExthData::default();
    let mut pos = 12;
    for _ in 0..count {
        if pos + 8 > total_len {
            break;
        }
        let rec_type = read_u32(slice, pos, "EXTH record type")?;
        let rec_len = read_u32(slice, pos + 4, "EXTH record length")? as usize;
        if rec_len < 8 || pos.saturating_add(rec_len) > total_len {
            break;
        }
        apply_record(
            &mut data,
            rec_type,
            &slice[pos + 8..pos + rec_len],
            encoding,
        );
        pos += rec_len;
    }
    Ok(data)
}

fn apply_record(data: &mut ExthData, rec_type: u32, payload: &[u8], encoding: TextEncoding) {
    let text = || -> Option<String> {
        let value = encoding.decode(trim_nul(payload));
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    };
    let u32_val = || {
        <[u8; 4]>::try_from(payload)
            .ok()
            .map(u32::from_be_bytes)
            .filter(|&value| value != NO_RECORD)
            .map(|value| value as usize)
    };
    match rec_type {
        100 => {
            if let Some(value) = text() {
                data.authors.push(value);
            }
        }
        101 if data.publisher.is_none() => data.publisher = text(),
        103 if data.description.is_none() => data.description = text(),
        104 => {
            if let Some(value) = text() {
                data.isbns.push(value);
            }
        }
        105 => {
            if let Some(value) = text()
                && !data.subjects.contains(&value)
            {
                data.subjects.push(value);
            }
        }
        106 if data.date.is_none() => data.date = text(),
        108 => {
            if let Some(value) = text() {
                data.contributors.push(value);
            }
        }
        121 => data.kf8_boundary = u32_val(),
        201 => data.cover_offset = u32_val(),
        202 => data.thumb_offset = u32_val(),
        503 if data.updated_title.is_none() => data.updated_title = text(),
        524 if data.language.is_none() => data.language = text(),
        _ => {}
    }
}

pub(super) fn build_metadata(
    exth: ExthData,
    full_name: Option<String>,
    pdb_name: &str,
    locale: Option<u32>,
) -> EpubMetadata {
    let title = exth.updated_title.or(full_name).or_else(|| {
        let trimmed = pdb_name.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    });
    let mut creators: Vec<EpubCreator> = exth
        .authors
        .into_iter()
        .map(|name| EpubCreator {
            name,
            role: Some("aut".to_owned()),
        })
        .collect();
    for name in exth.contributors {
        if !creators.iter().any(|existing| existing.name == name) {
            creators.push(EpubCreator { name, role: None });
        }
    }
    let language = exth
        .language
        .or_else(|| locale.and_then(locale_to_bcp47).map(str::to_owned));
    EpubMetadata {
        title,
        description: exth
            .description
            .as_deref()
            .map(strip_html_summary)
            .filter(|value| !value.is_empty()),
        date: exth.date,
        creators,
        identifiers: exth.isbns,
        collection: None,
        collection_position: None,
        publisher: exth.publisher,
        language,
        subjects: exth.subjects,
        page_progression_direction: None,
    }
}

fn strip_html_summary(html: &str) -> String {
    let mut text = String::new();
    let mut in_data = false;
    let mut emitter = DefaultEmitter::default();
    emitter.naively_switch_states(true);
    for token in Tokenizer::new_with_emitter(html, emitter).flatten() {
        match token {
            Token::StartTag(tag) => in_data = matches!(&tag.name[..], b"script" | b"style"),
            Token::EndTag(_) => in_data = false,
            Token::String(string) if !in_data => {
                text.push_str(&String::from_utf8_lossy(&string.value));
            }
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Maps the MOBI header primary language ID (Windows LCID low byte) to BCP-47.
fn locale_to_bcp47(locale: u32) -> Option<&'static str> {
    match locale & 0xff {
        0x01 => Some("ar"),
        0x04 => Some("zh"),
        0x06 => Some("da"),
        0x07 => Some("de"),
        0x08 => Some("el"),
        0x09 => Some("en"),
        0x0a => Some("es"),
        0x0b => Some("fi"),
        0x0c => Some("fr"),
        0x0d => Some("he"),
        0x0e => Some("hu"),
        0x10 => Some("it"),
        0x11 => Some("ja"),
        0x12 => Some("ko"),
        0x13 => Some("nl"),
        0x14 => Some("no"),
        0x15 => Some("pl"),
        0x16 => Some("pt"),
        0x18 => Some("ro"),
        0x19 => Some("ru"),
        0x1d => Some("sv"),
        0x1e => Some("th"),
        0x1f => Some("tr"),
        0x22 => Some("uk"),
        0x2a => Some("vi"),
        _ => None,
    }
}
