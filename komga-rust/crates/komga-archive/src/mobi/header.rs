//! PalmDB, PalmDOC, and MOBI header parsing.

use crate::epub::EpubMetadata;

use super::binary::{NO_RECORD, TextEncoding, decode_cp1252, read_u16, read_u32, trim_nul};
use super::exth::{self, ExthData};
use super::{MobiError, MobiUnsupportedReason};

const PALM_HEADER_LEN: usize = 78;
const PALMDOC_HEADER_LEN: usize = 16;
const EXTH_FLAG_PRESENT: u32 = 0x40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Compression {
    None,
    PalmDoc,
    HuffCdic,
}

/// Parsed PalmDB container and MOBI6 header metadata.
#[derive(Debug)]
pub(super) struct MobiHeader {
    pub(super) record_offsets: Vec<usize>,
    pub(super) compression: Compression,
    pub(super) text_record_count: usize,
    pub(super) text_encoding: TextEncoding,
    pub(super) first_image_index: Option<usize>,
    pub(super) huff_record_offset: usize,
    pub(super) huff_record_count: usize,
    pub(super) extra_record_data_flags: u32,
    pub(super) indx_record: Option<usize>,
    pub(super) kf8_boundary: Option<usize>,
    pub(super) cover_offset: Option<usize>,
    pub(super) thumb_offset: Option<usize>,
    pub(super) metadata: EpubMetadata,
    pub(super) unique_id: u32,
}

impl MobiHeader {
    pub(super) fn record<'a>(&self, bytes: &'a [u8], index: usize) -> Option<&'a [u8]> {
        let start = *self.record_offsets.get(index)?;
        let end = *self.record_offsets.get(index + 1)?;
        bytes.get(start..end)
    }

    pub(super) fn record_count(&self) -> usize {
        self.record_offsets.len().saturating_sub(1)
    }
}

/// Parses the PalmDB record table, PalmDOC header, MOBI header, and optional EXTH block.
pub(super) fn parse(bytes: &[u8]) -> Result<MobiHeader, MobiError> {
    if bytes.len() < PALM_HEADER_LEN || bytes.get(60..68) != Some(b"BOOKMOBI") {
        return Err(MobiError::Invalid("missing BOOKMOBI identifier".to_owned()));
    }
    let pdb_name = decode_cp1252(trim_nul(&bytes[..32]));
    let record_offsets = parse_record_offsets(bytes)?;
    let r0 = bytes
        .get(record_offsets[0]..record_offsets[1])
        .ok_or_else(|| MobiError::Invalid("record 0 out of bounds".to_owned()))?;
    if r0.len() < PALMDOC_HEADER_LEN + 24 {
        return Err(MobiError::Invalid("truncated record 0 header".to_owned()));
    }

    let (compression, text_record_count) = parse_palmdoc(r0, record_offsets.len())?;
    let fields = parse_mobi_fields(r0)?;
    let exth = if fields.exth_flags & EXTH_FLAG_PRESENT != 0 {
        exth::parse(
            &r0[PALMDOC_HEADER_LEN + fields.header_length..],
            fields.text_encoding,
        )?
    } else {
        ExthData::default()
    };

    let (kf8_boundary, cover_offset, thumb_offset) =
        (exth.kf8_boundary, exth.cover_offset, exth.thumb_offset);
    let metadata = exth::build_metadata(exth, fields.full_name, &pdb_name, fields.locale);

    Ok(MobiHeader {
        record_offsets,
        compression,
        text_record_count,
        text_encoding: fields.text_encoding,
        first_image_index: fields.first_image_index,
        huff_record_offset: fields.huff_record_offset,
        huff_record_count: fields.huff_record_count,
        extra_record_data_flags: fields.extra_record_data_flags,
        indx_record: fields.indx_record,
        kf8_boundary,
        cover_offset,
        thumb_offset,
        metadata,
        unique_id: fields.unique_id,
    })
}

fn parse_palmdoc(r0: &[u8], offset_count: usize) -> Result<(Compression, usize), MobiError> {
    let compression = match read_u16(r0, 0, "PalmDOC compression")? {
        1 => Compression::None,
        2 => Compression::PalmDoc,
        17480 => Compression::HuffCdic,
        other => {
            return Err(MobiError::Unsupported(MobiUnsupportedReason::Compression(
                other,
            )));
        }
    };
    let text_record_count = usize::from(read_u16(r0, 8, "text record count")?);
    if text_record_count == 0 || text_record_count + 1 >= offset_count {
        return Err(MobiError::Invalid("invalid text record count".to_owned()));
    }
    if read_u16(r0, 12, "encryption type")? != 0 {
        return Err(MobiError::Unsupported(MobiUnsupportedReason::Drm));
    }
    Ok((compression, text_record_count))
}

struct MobiFields {
    header_length: usize,
    text_encoding: TextEncoding,
    unique_id: u32,
    full_name: Option<String>,
    locale: Option<u32>,
    first_image_index: Option<usize>,
    huff_record_offset: usize,
    huff_record_count: usize,
    exth_flags: u32,
    extra_record_data_flags: u32,
    indx_record: Option<usize>,
}

fn parse_mobi_fields(r0: &[u8]) -> Result<MobiFields, MobiError> {
    let mobi = &r0[PALMDOC_HEADER_LEN..];
    if mobi.get(0..4) != Some(b"MOBI") {
        return Err(MobiError::Invalid("missing MOBI header magic".to_owned()));
    }
    let header_length = read_u32(mobi, 4, "MOBI header length")? as usize;
    if header_length < 24 || PALMDOC_HEADER_LEN.saturating_add(header_length) > r0.len() {
        return Err(MobiError::Invalid("invalid MOBI header length".to_owned()));
    }
    let mobi_type = read_u32(mobi, 8, "MOBI type")?;
    let file_version = read_u32(mobi, 20, "MOBI file version")?;
    if mobi_type == 0x0000_00f8 || file_version == 8 {
        return Err(MobiError::Unsupported(MobiUnsupportedReason::Kf8));
    }
    let text_encoding = match read_u32(mobi, 12, "MOBI text encoding")? {
        1252 => TextEncoding::Cp1252,
        65001 => TextEncoding::Utf8,
        other => {
            return Err(MobiError::Unsupported(MobiUnsupportedReason::Encoding(
                other,
            )));
        }
    };
    check_drm_fields(mobi, header_length)?;

    Ok(MobiFields {
        header_length,
        text_encoding,
        unique_id: read_u32(mobi, 16, "MOBI unique id")?,
        full_name: read_full_name(r0, mobi, header_length, text_encoding)?,
        locale: optional_u32(mobi, header_length, 76),
        first_image_index: optional_record(mobi, header_length, 92),
        huff_record_offset: optional_u32(mobi, header_length, 96).unwrap_or(0) as usize,
        huff_record_count: optional_u32(mobi, header_length, 100).unwrap_or(0) as usize,
        exth_flags: optional_u32(mobi, header_length, 112).unwrap_or(0),
        extra_record_data_flags: optional_u32(mobi, header_length, 224).unwrap_or(0),
        indx_record: optional_record(mobi, header_length, 228),
    })
}

fn parse_record_offsets(bytes: &[u8]) -> Result<Vec<usize>, MobiError> {
    let count = usize::from(read_u16(bytes, 76, "PalmDB record count")?);
    if count < 2 {
        return Err(MobiError::Invalid(
            "MOBI has fewer than 2 records".to_owned(),
        ));
    }
    let table_end = PALM_HEADER_LEN
        .checked_add(
            count
                .checked_mul(8)
                .ok_or_else(|| MobiError::Invalid("record table overflow".to_owned()))?,
        )
        .ok_or_else(|| MobiError::Invalid("record table overflow".to_owned()))?;
    if table_end > bytes.len() {
        return Err(MobiError::Invalid(
            "truncated PalmDB record table".to_owned(),
        ));
    }
    let mut offsets = Vec::with_capacity(count + 1);
    for index in 0..count {
        let offset = read_u32(bytes, PALM_HEADER_LEN + index * 8, "PalmDB record offset")? as usize;
        if offset > bytes.len() || offsets.last().is_some_and(|prev| *prev > offset) {
            return Err(MobiError::Invalid(
                "invalid PalmDB record offsets".to_owned(),
            ));
        }
        offsets.push(offset);
    }
    offsets.push(bytes.len());
    Ok(offsets)
}

fn check_drm_fields(mobi: &[u8], header_length: usize) -> Result<(), MobiError> {
    if header_length < 168 {
        return Ok(());
    }
    let drm_offset = read_u32(mobi, 152, "DRM offset")?;
    let drm_count = read_u32(mobi, 156, "DRM count")?;
    let drm_size = read_u32(mobi, 160, "DRM size")?;
    let drm_flags = read_u32(mobi, 164, "DRM flags")?;
    let has_offset = drm_offset != 0 && drm_offset != NO_RECORD;
    let has_count = drm_count != 0 && drm_count != NO_RECORD;
    if has_offset || has_count || drm_size != 0 || drm_flags != 0 {
        return Err(MobiError::Unsupported(MobiUnsupportedReason::Drm));
    }
    Ok(())
}

fn read_full_name(
    r0: &[u8],
    mobi: &[u8],
    header_length: usize,
    encoding: TextEncoding,
) -> Result<Option<String>, MobiError> {
    let (Some(offset), Some(length)) = (
        optional_u32(mobi, header_length, 68),
        optional_u32(mobi, header_length, 72),
    ) else {
        return Ok(None);
    };
    if length == 0 {
        return Ok(None);
    }
    let start = offset as usize;
    let end = start
        .checked_add(length as usize)
        .filter(|&end| end <= r0.len())
        .ok_or_else(|| MobiError::Invalid("MOBI full name out of bounds".to_owned()))?;
    let decoded = encoding.decode(&r0[start..end]);
    let trimmed = decoded.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
}

fn optional_u32(mobi: &[u8], header_length: usize, offset: usize) -> Option<u32> {
    if offset + 4 > header_length {
        return None;
    }
    read_u32(mobi, offset, "MOBI optional field").ok()
}

fn optional_record(mobi: &[u8], header_length: usize, offset: usize) -> Option<usize> {
    optional_u32(mobi, header_length, offset)
        .filter(|&value| value != NO_RECORD)
        .map(|value| value as usize)
}
