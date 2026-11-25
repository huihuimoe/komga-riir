//! MOBI INDX/NCX navigation table parsing (IDXT entries, TAGX tags, and CNCX labels).

use super::binary::{TextEncoding, read_u16, read_u32};
use super::header::MobiHeader;

const MAX_TOC_ENTRIES: usize = 16_384;
const MAX_INDEX_RECORDS: usize = 512;

/// One flat entry decoded from a MOBI NCX INDX table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IndxTocEntry {
    pub(super) offset: usize,
    pub(super) label: String,
    pub(super) depth: usize,
    pub(super) parent: Option<usize>,
}

/// Parses the NCX INDX table referenced by `header.indx_record`. Returns an empty list when no
/// valid NCX INDX is present so callers can fall back to `<guide>` or page breaks.
pub(super) fn parse_toc(bytes: &[u8], header: &MobiHeader) -> Vec<IndxTocEntry> {
    let Some(primary_index) = header.indx_record else {
        return Vec::new();
    };
    let Some(primary) = header.record(bytes, primary_index) else {
        return Vec::new();
    };
    let Some(meta) = parse_primary(primary, header.text_encoding) else {
        return Vec::new();
    };

    let cncx_start = primary_index + 1 + meta.index_count;
    let cncx_records: Vec<&[u8]> = (0..meta.cncx_count)
        .filter_map(|i| header.record(bytes, cncx_start + i))
        .collect();

    let mut entries = Vec::new();
    for offset in 0..meta.index_count {
        let Some(record) = header.record(bytes, primary_index + 1 + offset) else {
            break;
        };
        parse_secondary(record, &meta, &cncx_records, &mut entries);
        if entries.len() >= MAX_TOC_ENTRIES {
            break;
        }
    }
    entries
}

struct PrimaryIndx {
    index_count: usize,
    cncx_count: usize,
    encoding: TextEncoding,
    control_bytes: usize,
    tags: Vec<TagDef>,
}

#[derive(Clone, Copy)]
struct TagDef {
    id: u8,
    num_values: u8,
    bitmask: u8,
    end_of_control: bool,
}

fn parse_primary(record: &[u8], fallback_encoding: TextEncoding) -> Option<PrimaryIndx> {
    if record.len() < 56 || record.get(0..4) != Some(b"INDX") {
        return None;
    }
    let header_len = read_u32(record, 4, "INDX header len").ok()? as usize;
    let index_type = read_u32(record, 12, "INDX type").ok()?;
    let index_count = (read_u32(record, 24, "INDX count").ok()? as usize).min(MAX_INDEX_RECORDS);
    let encoding = match read_u32(record, 28, "INDX encoding").ok()? {
        1252 => TextEncoding::Cp1252,
        65001 => TextEncoding::Utf8,
        _ => fallback_encoding,
    };
    let cncx_count = (read_u32(record, 52, "INDX cncx count").ok()? as usize).min(64);
    if index_type != 0 || header_len + 12 > record.len() {
        return None;
    }
    let tagx = &record[header_len..];
    if tagx.get(0..4) != Some(b"TAGX") {
        return None;
    }
    let tagx_len = read_u32(tagx, 4, "TAGX len").ok()? as usize;
    let control_bytes = read_u32(tagx, 8, "TAGX control bytes").ok()? as usize;
    if tagx_len < 12 || tagx_len > tagx.len() || control_bytes == 0 || control_bytes > 16 {
        return None;
    }
    let mut tags = Vec::new();
    let mut pos = 12;
    while pos + 4 <= tagx_len {
        tags.push(TagDef {
            id: tagx[pos],
            num_values: tagx[pos + 1],
            bitmask: tagx[pos + 2],
            end_of_control: tagx[pos + 3] == 1,
        });
        pos += 4;
    }
    Some(PrimaryIndx {
        index_count,
        cncx_count,
        encoding,
        control_bytes,
        tags,
    })
}

fn parse_secondary(record: &[u8], meta: &PrimaryIndx, cncx: &[&[u8]], out: &mut Vec<IndxTocEntry>) {
    if record.len() < 32 || record.get(0..4) != Some(b"INDX") {
        return;
    }
    let Ok(idxt_offset) = read_u32(record, 20, "IDXT offset") else {
        return;
    };
    let Ok(entry_count) = read_u32(record, 24, "IDXT count") else {
        return;
    };
    let idxt = idxt_offset as usize;
    let count = (entry_count as usize).min(MAX_TOC_ENTRIES.saturating_sub(out.len()));
    if idxt + 4 + count * 2 > record.len() || record.get(idxt..idxt + 4) != Some(b"IDXT") {
        return;
    }
    let mut offsets = Vec::with_capacity(count + 1);
    for i in 0..count {
        let Ok(off) = read_u16(record, idxt + 4 + i * 2, "IDXT entry") else {
            return;
        };
        offsets.push(usize::from(off));
    }
    offsets.push(idxt);

    for window in offsets.windows(2) {
        let (start, end) = (window[0], window[1]);
        if start < end
            && end <= record.len()
            && let Some(entry) = parse_entry(&record[start..end], meta, cncx)
        {
            out.push(entry);
        }
    }
}

fn parse_entry(entry: &[u8], meta: &PrimaryIndx, cncx: &[&[u8]]) -> Option<IndxTocEntry> {
    let key_len = usize::from(*entry.first()?);
    let ctrl_end = 1 + key_len + meta.control_bytes;
    let controls = entry.get(1 + key_len..ctrl_end)?;
    let mut cursor = ctrl_end;
    let mut ctrl_index = 0;

    let mut pos_offset = None;
    let mut label_offset = None;
    let mut depth = 0;
    let mut parent = None;

    for tag in &meta.tags {
        if tag.end_of_control {
            ctrl_index += 1;
            continue;
        }
        let Some(&control) = controls.get(ctrl_index) else {
            break;
        };
        let masked = control & tag.bitmask;
        if masked == 0 {
            continue;
        }
        if let Some(first) = read_tag_first_value(entry, &mut cursor, *tag, masked)? {
            match tag.id {
                1 => pos_offset = Some(first),
                3 => label_offset = Some(first),
                4 => depth = first,
                21 => parent = Some(first),
                _ => {}
            }
        }
    }

    let offset = pos_offset?;
    let label = label_offset
        .and_then(|off| read_cncx_label(cncx, off, meta.encoding))
        .filter(|text| !text.trim().is_empty())?;
    Some(IndxTocEntry {
        offset,
        label: label.trim().to_owned(),
        depth,
        parent,
    })
}

/// Advances `cursor` past all VWI values for `tag` and returns the first value, if any.
/// Returns `None` if the entry is truncated.
fn read_tag_first_value(
    entry: &[u8],
    cursor: &mut usize,
    tag: TagDef,
    masked: u8,
) -> Option<Option<usize>> {
    if masked == tag.bitmask && tag.bitmask.count_ones() > 1 {
        let (vwi_len, used) = read_forward_vwi(&entry[*cursor..])?;
        *cursor += used;
        let end = cursor.checked_add(vwi_len)?.min(entry.len());
        let first = read_forward_vwi(&entry[*cursor..end]).map(|(value, _)| value);
        *cursor = end;
        return Some(first);
    }
    let count = if masked == tag.bitmask {
        usize::from(tag.num_values)
    } else {
        usize::from(masked >> tag.bitmask.trailing_zeros()) * usize::from(tag.num_values)
    };
    let mut first = None;
    for _ in 0..count {
        let (value, used) = read_forward_vwi(&entry[*cursor..])?;
        *cursor += used;
        first.get_or_insert(value);
    }
    Some(first)
}

fn read_forward_vwi(data: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0_usize;
    for (index, &byte) in data.iter().take(5).enumerate() {
        value = (value << 7) | usize::from(byte & 0x7f);
        if byte & 0x80 != 0 {
            return Some((value, index + 1));
        }
    }
    None
}

fn read_cncx_label(cncx: &[&[u8]], offset: usize, encoding: TextEncoding) -> Option<String> {
    let record_index = offset >> 16;
    let in_record = offset & 0xffff;
    let record = *cncx.get(record_index)?;
    let slice = record.get(in_record..)?;
    let (length, used) = read_forward_vwi(slice)?;
    let text = slice.get(used..used.checked_add(length)?)?;
    Some(encoding.decode(text))
}
