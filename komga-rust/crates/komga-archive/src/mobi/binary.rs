//! Big-endian field reads and PalmDB text decoding shared by the MOBI parsers.

use super::MobiError;

pub(super) const NO_RECORD: u32 = u32::MAX;

/// Windows-1252 `0x80..=0x9f` codepoints; undefined slots map to U+FFFD.
const CP1252_HIGH: [char; 32] = [
    '\u{20ac}', '\u{fffd}', '\u{201a}', '\u{0192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02c6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{fffd}', '\u{017d}', '\u{fffd}',
    '\u{fffd}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02dc}', '\u{2122}', '\u{0161}', '\u{203a}', '\u{0153}', '\u{fffd}', '\u{017e}', '\u{0178}',
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TextEncoding {
    Cp1252,
    Utf8,
}

impl TextEncoding {
    pub(super) fn decode(self, bytes: &[u8]) -> String {
        match self {
            Self::Cp1252 => decode_cp1252(bytes),
            Self::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
        }
    }
}

pub(super) fn decode_cp1252(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        match byte {
            0x80..=0x9f => out.push(CP1252_HIGH[usize::from(byte - 0x80)]),
            other => out.push(char::from(other)),
        }
    }
    out
}

pub(super) fn trim_nul(bytes: &[u8]) -> &[u8] {
    match bytes.iter().position(|&byte| byte == 0) {
        Some(pos) => &bytes[..pos],
        None => bytes,
    }
}

pub(super) fn read_u16(bytes: &[u8], offset: usize, field: &str) -> Result<u16, MobiError> {
    let slice = bytes
        .get(offset..offset.saturating_add(2))
        .filter(|slice| slice.len() == 2)
        .ok_or_else(|| MobiError::Invalid(format!("truncated {field}")))?;
    Ok(u16::from_be_bytes([slice[0], slice[1]]))
}

pub(super) fn read_u32(bytes: &[u8], offset: usize, field: &str) -> Result<u32, MobiError> {
    let slice = bytes
        .get(offset..offset.saturating_add(4))
        .filter(|slice| slice.len() == 4)
        .ok_or_else(|| MobiError::Invalid(format!("truncated {field}")))?;
    Ok(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}
