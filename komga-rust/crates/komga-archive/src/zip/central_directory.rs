//! commons-compress 1.28 `ZipFile` opening: the end of central directory record, ZIP64, the
//! central directory entries and, unless ignored, every local file header.

use std::io::{self, Read, Seek, SeekFrom};

use super::{Entry, ZipError, extra_field, java_utf8};

const END_SIGNATURE: [u8; 4] = *b"PK\x05\x06";
const ZIP64_LOCATOR_SIGNATURE: [u8; 4] = *b"PK\x06\x07";
const ZIP64_END_SIGNATURE: [u8; 4] = *b"PK\x06\x06";
const CENTRAL_SIGNATURE: [u8; 4] = *b"PK\x01\x02";
const LOCAL_SIGNATURE: [u8; 4] = *b"PK\x03\x04";
const END_LENGTH: u64 = 22;
const MAX_COMMENT_LENGTH: u64 = 0xffff;
const ZIP64_LOCATOR_LENGTH: u64 = 20;
const ZIP64_MAGIC: u64 = 0xffff_ffff;
const FAT_PLATFORM: u16 = 0;
const UTF8_FLAG: u16 = 1 << 11;
/// Offset of the name length in a local file header, and the fixed header length.
const LOCAL_NAME_LENGTH_OFFSET: u64 = 26;
const LOCAL_HEADER_LENGTH: u64 = 30;
/// Limits on what the directory holds in memory until the archive is closed.
const MAX_ENTRIES: usize = 100_000;
const MAX_NAME_AND_EXTRA_BYTES: u64 = 64 * 1024 * 1024;

pub(super) struct CentralDirectory {
    pub(super) entries: Vec<Entry>,
    pub(super) offset: u64,
}

/// A central directory entry still waiting for its local file header.
struct Record {
    entry: Entry,
    central_extra: Vec<u8>,
    /// The raw name, for an entry without the UTF-8 flag that a Unicode Path may rename.
    renamable: Option<Vec<u8>>,
    fat: bool,
}

pub(super) fn read(
    reader: &mut (impl Read + Seek),
    local_headers: bool,
) -> Result<CentralDirectory, ZipError> {
    let (offset, first_local_header) = locate(reader)?;
    let mut records = Vec::new();
    seek(reader, offset)?;
    let mut signature = [0; 4];
    reader.read_exact(&mut signature)?;
    if signature != CENTRAL_SIGNATURE {
        seek(reader, first_local_header)?;
        reader.read_exact(&mut signature)?;
        if signature == LOCAL_SIGNATURE {
            return Err(
                invalid("Central directory is empty, can't expand corrupt archive.").into(),
            );
        }
    }
    let mut name_and_extra_budget = MAX_NAME_AND_EXTRA_BYTES;
    while signature == CENTRAL_SIGNATURE {
        if records.len() == MAX_ENTRIES {
            return Err(ZipError::LimitExceeded(
                "Archive lists more than 100000 entries",
            ));
        }
        records.push(read_record(
            reader,
            first_local_header,
            offset,
            &mut name_and_extra_budget,
        )?);
        reader.read_exact(&mut signature)?;
    }
    let mut entries = Vec::with_capacity(records.len());
    for mut record in records {
        if local_headers {
            let local_extra = resolve_local_header(reader, &mut record.entry, offset)?;
            rename(&mut record, &local_extra);
        }
        entries.push(record.entry);
    }
    Ok(CentralDirectory { entries, offset })
}

/// Positions of the first central directory entry and of the first local file header, which
/// is past any data prepended to a ZIP archive without ZIP64 records.
fn locate(reader: &mut (impl Read + Seek)) -> io::Result<(u64, u64)> {
    let end = find_end(reader)?;
    let mut locator = [0; ZIP64_LOCATOR_LENGTH as usize];
    let zip64 = end > ZIP64_LOCATOR_LENGTH && {
        seek(reader, end - ZIP64_LOCATOR_LENGTH)?;
        reader.read_exact(&mut locator)?;
        locator[..4] == ZIP64_LOCATOR_SIGNATURE
    };
    if zip64 {
        // Komga opens archives that span one disk at most.
        if u32_at(&locator, 16) > 1 {
            return Err(invalid("Too many disks for zip archive"));
        }
        seek(reader, signed(u64_at(&locator, 8))?)?;
        let mut record = [0; 56];
        reader.read_exact(&mut record)?;
        if record[..4] != ZIP64_END_SIGNATURE {
            return Err(invalid(
                "Archive's ZIP64 end of central directory locator is corrupt.",
            ));
        }
        return Ok((signed(u64_at(&record, 48))?, 0));
    }
    let mut record = [0; END_LENGTH as usize];
    seek(reader, end)?;
    reader.read_exact(&mut record)?;
    if u16_at(&record, 4) != 0 {
        return Err(invalid("Too many disks for zip archive"));
    }
    let length = u64::from(u32_at(&record, 12));
    let offset = u64::from(u32_at(&record, 16));
    let first_local_header = end.saturating_sub(length).saturating_sub(offset);
    Ok((offset + first_local_header, first_local_header))
}

/// The last end of central directory signature that leaves room for the record, within the
/// longest possible archive comment of the end.
fn find_end(reader: &mut (impl Read + Seek)) -> io::Result<u64> {
    let size = reader.seek(SeekFrom::End(0))?;
    let not_a_zip = || invalid("Archive is not a ZIP archive");
    let last = size.checked_sub(END_LENGTH).ok_or_else(not_a_zip)?;
    let first = size.saturating_sub(END_LENGTH + MAX_COMMENT_LENGTH);
    let mut tail = Vec::new();
    seek(reader, first)?;
    reader.read_to_end(&mut tail)?;
    let candidates = usize::try_from(last - first).expect("Within the tail") + 4;
    let position = tail[..candidates]
        .windows(4)
        .rposition(|window| window == END_SIGNATURE)
        .ok_or_else(not_a_zip)?;
    Ok(first + position as u64)
}

/// Reads one record, whose name and extra field are taken from `name_and_extra_budget` before
/// they are allocated.
fn read_record(
    reader: &mut impl Read,
    first_local_header: u64,
    central_directory: u64,
    name_and_extra_budget: &mut u64,
) -> Result<Record, ZipError> {
    let mut header = [0; 42];
    reader.read_exact(&mut header)?;
    let stored = u64::from(u16_at(&header, 24)) + u64::from(u16_at(&header, 26));
    *name_and_extra_budget =
        name_and_extra_budget
            .checked_sub(stored)
            .ok_or(ZipError::LimitExceeded(
                "Archive names and extra fields exceed 64 MiB",
            ))?;
    let flags = u16_at(&header, 4);
    let raw_name = read_vec(reader, u16_at(&header, 24))?;
    let central_extra = read_vec(reader, u16_at(&header, 26))?;
    let fat = (u16_at(&header, 0) >> 8) & 0x0f == FAT_PLATFORM;
    let mut entry = Entry {
        name: name(java_utf8::decode(&raw_name), fat),
        size: u64::from(u32_at(&header, 20)),
        method: u16_at(&header, 6),
        flags,
        compressed_size: u64::from(u32_at(&header, 16)),
        local_header_offset: u64::from(u32_at(&header, 38)) + first_local_header,
        data_offset: None,
    };
    if let Some(zip64) = extra_field::zip64(&central_extra) {
        apply_zip64(&mut entry, u16_at(&header, 30), zip64)?;
    }
    if entry.local_header_offset > central_directory {
        return Err(invalid(format!(
            "local file header for {} starts after central directory",
            entry.name
        ))
        .into());
    }
    read_vec(reader, u16_at(&header, 28))?;
    Ok(Record {
        entry,
        central_extra,
        renamable: (flags & UTF8_FLAG == 0).then_some(raw_name),
        fat,
    })
}

/// `setSizesAndOffsetFromZip64Extra`: the field holds, in order, each value whose header
/// field is saturated.
fn apply_zip64(entry: &mut Entry, disk_start: u16, zip64: &[u8]) -> io::Result<()> {
    let has_size = entry.size == ZIP64_MAGIC;
    let has_compressed_size = entry.compressed_size == ZIP64_MAGIC;
    // Compared after adding the prepended data length, as Komga does.
    let has_offset = entry.local_header_offset == ZIP64_MAGIC;
    let expected = 8 * usize::from(has_size)
        + 8 * usize::from(has_compressed_size)
        + 8 * usize::from(has_offset)
        + 4 * usize::from(disk_start == 0xffff);
    if zip64.len() < expected {
        return Err(invalid(format!(
            "Central directory zip64 extended information extra field's length doesn't match central directory data.  Expected length {expected} but is {}",
            zip64.len()
        )));
    }
    let mut offset = 0;
    let mut next = |present: bool, target: &mut u64| -> io::Result<()> {
        if present {
            *target = signed(u64_at(zip64, offset))?;
            offset += 8;
        }
        Ok(())
    };
    next(has_size, &mut entry.size)?;
    next(has_compressed_size, &mut entry.compressed_size)?;
    next(has_offset, &mut entry.local_header_offset)
}

/// `setDataOffset` and the local extra fields, with the bounds `ZipFile` checks.
pub(super) fn resolve_local_header(
    reader: &mut (impl Read + Seek),
    entry: &mut Entry,
    central_directory: u64,
) -> io::Result<Vec<u8>> {
    seek(reader, entry.local_header_offset + LOCAL_NAME_LENGTH_OFFSET)?;
    let mut lengths = [0; 4];
    reader.read_exact(&mut lengths)?;
    let name_length = u64::from(u16_at(&lengths, 0));
    let data_offset = entry.local_header_offset
        + LOCAL_HEADER_LENGTH
        + name_length
        + u64::from(u16_at(&lengths, 2));
    if data_offset + entry.compressed_size > central_directory {
        return Err(invalid(format!(
            "data for {} overlaps with central directory.",
            entry.name
        )));
    }
    entry.data_offset = Some(data_offset);
    reader.seek(SeekFrom::Current(
        i64::try_from(name_length).expect("A u16"),
    ))?;
    read_vec(reader, u16_at(&lengths, 2))
}

/// `setNameAndCommentFromExtraFields`: a Unicode Path for the raw name replaces it.
fn rename(record: &mut Record, local_extra: &[u8]) {
    let Some(raw_name) = &record.renamable else {
        return;
    };
    let unicode_path = extra_field::unicode_path(&record.central_extra, local_extra);
    if let Some(path) = unicode_path.filter(|path| path.name_crc == crc32fast::hash(raw_name)) {
        record.entry.name = name(java_utf8::decode(path.name), record.fat);
    }
}

/// `ZipArchiveEntry.setName`: FAT names without `/` use `\` as separator.
fn name(name: String, fat: bool) -> String {
    if fat && !name.contains('/') {
        name.replace('\\', "/")
    } else {
        name
    }
}

/// Java `long` positions: larger unsigned values are negative and rejected.
fn signed(value: u64) -> io::Result<u64> {
    i64::try_from(value)
        .map(|_| value)
        .map_err(|_| invalid("broken archive, negative value"))
}

fn seek(reader: &mut impl Seek, position: u64) -> io::Result<()> {
    reader.seek(SeekFrom::Start(position)).map(drop)
}

fn read_vec(reader: &mut impl Read, length: u16) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; usize::from(length)];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("Four bytes"))
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("Eight bytes"))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
