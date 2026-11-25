//! MOBI text record decompression: uncompressed, PalmDOC LZ77, and HUFF/CDIC (`17480`).

use super::binary::{read_u16, read_u32};
use super::header::{Compression, MobiHeader};
use super::{MAX_TEXT_BYTES, MobiError};

const MAX_HUFF_DEPTH: usize = 32;
const MAX_CDIC_RECORDS: usize = 256;

enum TextDecoder {
    None,
    PalmDoc,
    HuffCdic(Box<HuffCdic>),
}

/// Decompresses all text records into raw decoded text bytes, respecting [`MAX_TEXT_BYTES`].
pub(super) fn read_text(bytes: &[u8], header: &MobiHeader) -> Result<String, MobiError> {
    let decoder = match header.compression {
        Compression::None => TextDecoder::None,
        Compression::PalmDoc => TextDecoder::PalmDoc,
        Compression::HuffCdic => TextDecoder::HuffCdic(Box::new(HuffCdic::parse(bytes, header)?)),
    };

    let mut raw = Vec::new();
    for index in 1..=header.text_record_count {
        let record = header
            .record(bytes, index)
            .ok_or_else(|| MobiError::Invalid("text record out of bounds".to_owned()))?;
        let trimmed = strip_trailing_entries(record, header.extra_record_data_flags);
        match &decoder {
            TextDecoder::None => {
                check_budget(raw.len(), trimmed.len())?;
                raw.extend_from_slice(trimmed);
            }
            TextDecoder::PalmDoc => uncompress_palmdoc(trimmed, &mut raw)?,
            TextDecoder::HuffCdic(huff) => huff.decompress(trimmed, &mut raw, 0)?,
        }
    }
    Ok(header.text_encoding.decode(&raw))
}

fn check_budget(current: usize, add: usize) -> Result<(), MobiError> {
    if current.saturating_add(add) > MAX_TEXT_BYTES {
        return Err(MobiError::LimitExceeded(
            "decompressed MOBI text exceeds limit",
        ));
    }
    Ok(())
}

/// Strips trailing extra-record-data entries indicated by `flags` (bit 0 = multibyte overlap,
/// higher bits = backward-VWI sized trailing entries).
pub(super) fn strip_trailing_entries(record: &[u8], flags: u32) -> &[u8] {
    let mut remaining = record.len();
    for _ in 0..(flags >> 1).count_ones() {
        let Some(size) = read_backward_vwi(&record[..remaining]) else {
            break;
        };
        if size == 0 || size > remaining {
            break;
        }
        remaining -= size;
    }
    if flags & 1 != 0 && remaining > 0 {
        let overlap = usize::from(record[remaining - 1] & 0b11) + 1;
        if overlap <= remaining {
            remaining -= overlap;
        }
    }
    &record[..remaining]
}

fn read_backward_vwi(data: &[u8]) -> Option<usize> {
    let start = data.len().saturating_sub(4);
    let mut value = 0_usize;
    let mut found = false;
    for &byte in &data[start..] {
        if byte & 0x80 != 0 {
            value = 0;
            found = true;
        }
        value = (value << 7) | usize::from(byte & 0x7f);
    }
    found.then_some(value)
}

fn uncompress_palmdoc(data: &[u8], output: &mut Vec<u8>) -> Result<(), MobiError> {
    let mut offset = 0;
    while offset < data.len() {
        let byte = data[offset];
        offset += 1;
        match byte {
            0 => {
                check_budget(output.len(), 1)?;
                output.push(0);
            }
            1..=8 => {
                let count = usize::from(byte);
                let end = offset
                    .checked_add(count)
                    .filter(|&end| end <= data.len())
                    .ok_or_else(|| MobiError::Invalid("truncated PalmDOC literal".to_owned()))?;
                check_budget(output.len(), count)?;
                output.extend_from_slice(&data[offset..end]);
                offset = end;
            }
            9..=127 => {
                check_budget(output.len(), 1)?;
                output.push(byte);
            }
            128..=191 => {
                let next = *data.get(offset).ok_or_else(|| {
                    MobiError::Invalid("truncated PalmDOC back-reference".to_owned())
                })?;
                offset += 1;
                let distance = (((usize::from(byte) << 8) | usize::from(next)) >> 3) & 0x7ff;
                let length = usize::from(next & 0x7) + 3;
                if distance == 0 || distance > output.len() {
                    return Err(MobiError::Invalid(
                        "invalid PalmDOC back-reference".to_owned(),
                    ));
                }
                check_budget(output.len(), length)?;
                for _ in 0..length {
                    let index = output.len() - distance;
                    output.push(output[index]);
                }
            }
            _ => {
                check_budget(output.len(), 2)?;
                output.push(b' ');
                output.push(byte ^ 0x80);
            }
        }
    }
    Ok(())
}

/// MOBI HUFF/CDIC dictionary & Huffman decoder (`compression == 17480`).
struct HuffCdic {
    dict1: [(u32, u8, bool); 256],
    mincode: [u32; 33],
    maxcode_raw: [u32; 33],
    dictionary: Vec<(Vec<u8>, bool)>,
}

impl HuffCdic {
    fn parse(bytes: &[u8], header: &MobiHeader) -> Result<Self, MobiError> {
        if header.huff_record_count < 2 || header.huff_record_count > MAX_CDIC_RECORDS + 1 {
            return Err(MobiError::Invalid(
                "invalid HUFF/CDIC record count".to_owned(),
            ));
        }
        let huff = header
            .record(bytes, header.huff_record_offset)
            .ok_or_else(|| MobiError::Invalid("missing HUFF record".to_owned()))?;
        let (dict1, mincode, maxcode_raw) = parse_huff_record(huff)?;

        let mut dictionary = Vec::new();
        for offset in 1..header.huff_record_count {
            let cdic = header
                .record(bytes, header.huff_record_offset + offset)
                .ok_or_else(|| MobiError::Invalid("missing CDIC record".to_owned()))?;
            parse_cdic_record(cdic, &mut dictionary)?;
        }
        Ok(Self {
            dict1,
            mincode,
            maxcode_raw,
            dictionary,
        })
    }

    fn decompress(&self, data: &[u8], output: &mut Vec<u8>, depth: usize) -> Result<(), MobiError> {
        if depth > MAX_HUFF_DEPTH {
            return Err(MobiError::Invalid(
                "HUFF/CDIC recursion limit exceeded".to_owned(),
            ));
        }
        let total_bits = data.len().saturating_mul(8);
        let mut bit_pos = 0_usize;
        while bit_pos < total_bits {
            let code = peek_u32_bits(data, bit_pos);
            let (mut max_raw, mut codelen, term) = self.dict1[(code >> 24) as usize];
            if !term {
                while (codelen as usize) < 32 && code < self.mincode[codelen as usize] {
                    codelen += 1;
                }
                max_raw = self.maxcode_raw[codelen as usize];
            }
            if !(1..=32).contains(&codelen) {
                return Err(MobiError::Invalid(
                    "invalid HUFF/CDIC code length".to_owned(),
                ));
            }
            let shift = 32 - u32::from(codelen);
            let value = max_raw.wrapping_sub(code >> shift);
            bit_pos += usize::from(codelen);
            if bit_pos > total_bits {
                break;
            }
            let (slice, is_terminal) = self.dictionary.get(value as usize).ok_or_else(|| {
                MobiError::Invalid("HUFF/CDIC dictionary index out of bounds".to_owned())
            })?;
            if *is_terminal {
                check_budget(output.len(), slice.len())?;
                output.extend_from_slice(slice);
            } else {
                self.decompress(slice, output, depth + 1)?;
            }
        }
        Ok(())
    }
}

type HuffTables = ([(u32, u8, bool); 256], [u32; 33], [u32; 33]);

fn parse_huff_record(huff: &[u8]) -> Result<HuffTables, MobiError> {
    if huff.len() < 24 || huff.get(0..8) != Some(b"HUFF\x00\x00\x00\x18") {
        return Err(MobiError::Invalid("invalid HUFF record header".to_owned()));
    }
    let off1 = read_u32(huff, 8, "HUFF table1 offset")? as usize;
    let off2 = read_u32(huff, 12, "HUFF table2 offset")? as usize;
    if off1.saturating_add(256 * 4) > huff.len() || off2.saturating_add(64 * 4) > huff.len() {
        return Err(MobiError::Invalid("truncated HUFF tables".to_owned()));
    }
    let mut dict1 = [(0_u32, 0_u8, false); 256];
    for (index, slot) in dict1.iter_mut().enumerate() {
        let raw = read_u32(huff, off1 + index * 4, "HUFF dict1 entry")?;
        let codelen = (raw & 0x1f) as u8;
        let term = raw & 0x80 != 0;
        let max_raw = raw >> 8;
        *slot = (max_raw, codelen, term);
    }
    let mut mincode = [0_u32; 33];
    let mut maxcode_raw = [0_u32; 33];
    for len in 1..=32 {
        let base = off2 + (len - 1) * 8;
        let min_raw = read_u32(huff, base, "HUFF mincode")?;
        let max_raw = read_u32(huff, base + 4, "HUFF maxcode")?;
        mincode[len] = min_raw.wrapping_shl((32 - len) as u32);
        maxcode_raw[len] = max_raw;
    }
    Ok((dict1, mincode, maxcode_raw))
}

fn parse_cdic_record(cdic: &[u8], dictionary: &mut Vec<(Vec<u8>, bool)>) -> Result<(), MobiError> {
    if cdic.len() < 16 || cdic.get(0..8) != Some(b"CDIC\x00\x00\x00\x10") {
        return Err(MobiError::Invalid("invalid CDIC record header".to_owned()));
    }
    let phrases = read_u32(cdic, 8, "CDIC phrase count")? as usize;
    let bits = read_u32(cdic, 12, "CDIC code bits")?;
    if bits == 0 || bits > 16 {
        return Err(MobiError::Invalid("invalid CDIC bit width".to_owned()));
    }
    let remaining = phrases.saturating_sub(dictionary.len());
    let entries_in_record = remaining.min(1_usize << bits);
    let data = &cdic[16..];
    if entries_in_record.saturating_mul(2) > data.len() {
        return Err(MobiError::Invalid("truncated CDIC offset table".to_owned()));
    }
    for index in 0..entries_in_record {
        let offset = usize::from(read_u16(data, index * 2, "CDIC entry offset")?);
        let header = read_u16(data, offset, "CDIC entry length")?;
        let length = usize::from(header & 0x7fff);
        let is_terminal = header & 0x8000 != 0;
        let start = offset + 2;
        let end = start
            .checked_add(length)
            .filter(|&end| end <= data.len())
            .ok_or_else(|| MobiError::Invalid("CDIC entry out of bounds".to_owned()))?;
        dictionary.push((data[start..end].to_vec(), is_terminal));
    }
    Ok(())
}

/// Reads the next 32 bits starting at `bit_pos`, zero-padded past end of `data`.
fn peek_u32_bits(data: &[u8], bit_pos: usize) -> u32 {
    let byte_pos = bit_pos / 8;
    let bit_shift = (bit_pos % 8) as u32;
    let mut acc = 0_u64;
    for i in 0..5 {
        acc = (acc << 8) | u64::from(data.get(byte_pos + i).copied().unwrap_or(0));
    }
    ((acc << bit_shift) >> 8) as u32
}
