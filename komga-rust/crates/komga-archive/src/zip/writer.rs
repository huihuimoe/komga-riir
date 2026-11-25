//! A ZIP archive written front to back, as commons-compress `ZipArchiveOutputStream` writes one
//! to a non-seekable stream with `DEFLATED` at `NO_COMPRESSION` and `Zip64Mode.Always`: entries
//! are deflated in stored blocks, followed by a ZIP64 data descriptor, and the central directory
//! closes with ZIP64 end records. Names are UTF-8 with the language encoding flag.
//!
//! Each call returns the bytes to append to the archive, so callers stream it with any I/O.
//! [`ZipStreamWriter::with_compression`] deflates for real, and small entries can be stored.

use std::io::Write;

use crc32fast::Hasher;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use jiff::civil::DateTime;

const VERSION: u16 = 45;
/// UTF-8 names.
const UTF8: u16 = 0x0800;
/// Sizes in a data descriptor, UTF-8 names.
const FLAGS: u16 = 0x0008 | UTF8;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;
const ZIP64_EXTRA: u16 = 0x0001;

/// An entry being written.
struct OpenEntry {
    encoder: DeflateEncoder<Vec<u8>>,
    crc: Hasher,
    compressed_size: u64,
}

/// An entry written, for the central directory.
struct WrittenEntry {
    name: String,
    flags: u16,
    method: u16,
    crc: u32,
    size: u64,
    compressed_size: u64,
    local_header_offset: u64,
}

pub struct ZipStreamWriter {
    compression: Compression,
    dos_time: u16,
    dos_date: u16,
    offset: u64,
    entries: Vec<WrittenEntry>,
    current: Option<(OpenEntry, WrittenEntry)>,
}

impl ZipStreamWriter {
    /// Every entry is dated `modified`, clamped to the DOS range.
    #[must_use]
    pub fn new(modified: DateTime) -> Self {
        Self::with_compression(modified, Compression::none())
    }

    /// Deflates entries at `compression`.
    #[must_use]
    pub fn with_compression(modified: DateTime, compression: Compression) -> Self {
        let (dos_time, dos_date) = dos_date_time(modified);
        Self {
            compression,
            dos_time,
            dos_date,
            offset: 0,
            entries: Vec::new(),
            current: None,
        }
    }

    /// Starts the entry `name`, returning its local file header.
    ///
    /// # Panics
    ///
    /// If an entry is already open.
    pub fn start_entry(&mut self, name: &str) -> Vec<u8> {
        assert!(
            self.current.is_none(),
            "Finish the open entry before starting another"
        );
        let mut header = Vec::with_capacity(50 + name.len());
        put_u32(&mut header, 0x0403_4b50);
        put_u16(&mut header, VERSION);
        self.put_common(&mut header, FLAGS, DEFLATED, 0, name, 20);
        header.extend_from_slice(name.as_bytes());
        put_u16(&mut header, ZIP64_EXTRA);
        put_u16(&mut header, 16);
        put_u64(&mut header, 0);
        put_u64(&mut header, 0);
        let entry = WrittenEntry {
            name: name.to_owned(),
            flags: FLAGS,
            method: DEFLATED,
            crc: 0,
            size: 0,
            compressed_size: 0,
            local_header_offset: self.offset,
        };
        let open = OpenEntry {
            encoder: DeflateEncoder::new(Vec::new(), self.compression),
            crc: Hasher::new(),
            compressed_size: 0,
        };
        self.current = Some((open, entry));
        self.emit(header)
    }

    /// Writes the whole entry `name` uncompressed, with its sizes in the local header, as the
    /// EPUB `mimetype` entry must be.
    ///
    /// # Panics
    ///
    /// If an entry is open or `data` does not fit 32-bit sizes.
    pub fn stored_entry(&mut self, name: &str, data: &[u8]) -> Vec<u8> {
        assert!(
            self.current.is_none(),
            "Finish the open entry before starting another"
        );
        let size = u32::try_from(data.len()).expect("Stored entries are small");
        let crc = crc32fast::hash(data);
        let mut out = Vec::with_capacity(30 + name.len() + data.len());
        put_u32(&mut out, 0x0403_4b50);
        put_u16(&mut out, VERSION);
        put_u16(&mut out, UTF8);
        put_u16(&mut out, STORED);
        put_u16(&mut out, self.dos_time);
        put_u16(&mut out, self.dos_date);
        put_u32(&mut out, crc);
        put_u32(&mut out, size);
        put_u32(&mut out, size);
        put_u16(
            &mut out,
            u16::try_from(name.len()).expect("Entry names fit a ZIP header"),
        );
        put_u16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        self.entries.push(WrittenEntry {
            name: name.to_owned(),
            flags: UTF8,
            method: STORED,
            crc,
            size: data.len() as u64,
            compressed_size: data.len() as u64,
            local_header_offset: self.offset,
        });
        self.emit(out)
    }

    /// Adds `data` to the open entry, returning what is ready of its deflated form.
    ///
    /// # Panics
    ///
    /// Without an open entry.
    pub fn write(&mut self, data: &[u8]) -> Vec<u8> {
        let (open, entry) = self
            .current
            .as_mut()
            .expect("Start an entry before writing");
        open.crc.update(data);
        entry.size += data.len() as u64;
        open.encoder
            .write_all(data)
            .expect("Writing to a vector does not fail");
        let out = std::mem::take(open.encoder.get_mut());
        open.compressed_size += out.len() as u64;
        self.emit(out)
    }

    /// Ends the open entry, returning the end of its data and its data descriptor.
    ///
    /// # Panics
    ///
    /// Without an open entry.
    pub fn finish_entry(&mut self) -> Vec<u8> {
        let (open, mut entry) = self
            .current
            .take()
            .expect("Start an entry before finishing it");
        let mut out = open
            .encoder
            .finish()
            .expect("Writing to a vector does not fail");
        entry.compressed_size = open.compressed_size + out.len() as u64;
        entry.crc = open.crc.finalize();
        put_u32(&mut out, 0x0807_4b50);
        put_u32(&mut out, entry.crc);
        put_u64(&mut out, entry.compressed_size);
        put_u64(&mut out, entry.size);
        self.entries.push(entry);
        self.emit(out)
    }

    /// Ends the archive, returning its central directory and end records.
    ///
    /// # Panics
    ///
    /// With an open entry.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        assert!(
            self.current.is_none(),
            "Finish the open entry before the archive"
        );
        let mut out = Vec::new();
        for entry in &self.entries {
            put_u32(&mut out, 0x0201_4b50);
            put_u16(&mut out, VERSION);
            put_u16(&mut out, VERSION);
            self.put_common(
                &mut out,
                entry.flags,
                entry.method,
                entry.crc,
                &entry.name,
                28,
            );
            // Comment length, disk number, internal and external attributes.
            out.extend_from_slice(&[0; 10]);
            put_u32(&mut out, u32::MAX);
            out.extend_from_slice(entry.name.as_bytes());
            put_u16(&mut out, ZIP64_EXTRA);
            put_u16(&mut out, 24);
            put_u64(&mut out, entry.size);
            put_u64(&mut out, entry.compressed_size);
            put_u64(&mut out, entry.local_header_offset);
        }
        let directory_offset = self.offset;
        let directory_size = out.len() as u64;
        let zip64_end_offset = directory_offset + directory_size;
        let count = self.entries.len() as u64;
        put_u32(&mut out, 0x0606_4b50);
        put_u64(&mut out, 44);
        put_u16(&mut out, VERSION);
        put_u16(&mut out, VERSION);
        put_u32(&mut out, 0);
        put_u32(&mut out, 0);
        put_u64(&mut out, count);
        put_u64(&mut out, count);
        put_u64(&mut out, directory_size);
        put_u64(&mut out, directory_offset);
        put_u32(&mut out, 0x0706_4b50);
        put_u32(&mut out, 0);
        put_u64(&mut out, zip64_end_offset);
        put_u32(&mut out, 1);
        put_u32(&mut out, 0x0605_4b50);
        put_u32(&mut out, 0);
        put_u16(&mut out, u16::MAX);
        put_u16(&mut out, u16::MAX);
        put_u32(&mut out, u32::MAX);
        put_u32(&mut out, u32::MAX);
        put_u16(&mut out, 0);
        out
    }

    /// The header fields from the flags to the extra field length, which local and central
    /// headers share; both sizes are in the ZIP64 extra field.
    fn put_common(
        &self,
        out: &mut Vec<u8>,
        flags: u16,
        method: u16,
        crc: u32,
        name: &str,
        extra_length: u16,
    ) {
        put_u16(out, flags);
        put_u16(out, method);
        put_u16(out, self.dos_time);
        put_u16(out, self.dos_date);
        put_u32(out, crc);
        put_u32(out, u32::MAX);
        put_u32(out, u32::MAX);
        put_u16(
            out,
            u16::try_from(name.len()).expect("Entry names fit a ZIP header"),
        );
        put_u16(out, extra_length);
    }

    fn emit(&mut self, bytes: Vec<u8>) -> Vec<u8> {
        self.offset += bytes.len() as u64;
        bytes
    }
}

/// MS-DOS time and date, which count years from 1980 to 2107 and seconds by two.
fn dos_date_time(modified: DateTime) -> (u16, u16) {
    let field = |value: i8| u16::from(value.unsigned_abs());
    let year = modified.year().clamp(1980, 2107).unsigned_abs() - 1980;
    let time = (field(modified.hour()) << 11)
        | (field(modified.minute()) << 5)
        | (field(modified.second()) / 2);
    let date = (year << 9) | (field(modified.month()) << 5) | field(modified.day());
    (time, date)
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};

    use flate2::Compression;
    use jiff::civil::date;

    use super::ZipStreamWriter;
    use crate::zip::ZipArchive;

    #[test]
    fn entries_read_back_in_order_with_their_content() {
        let large: Vec<u8> = (0..200_000_u32).map(|i| (i % 251) as u8).collect();
        let modified = date(2024, 5, 6).at(7, 8, 10, 0);
        for compression in [Compression::none(), Compression::default()] {
            let mut writer = ZipStreamWriter::with_compression(modified, compression);
            let mut archive = writer.stored_entry("mimetype", b"application/epub+zip");
            archive.extend(writer.start_entry("1 - été.cbz"));
            for chunk in large.chunks(65_536) {
                archive.extend(writer.write(chunk));
            }
            archive.extend(writer.finish_entry());
            archive.extend(writer.start_entry("2 - empty.cbz"));
            archive.extend(writer.finish_entry());
            archive.extend(writer.finish());
            // EPUB readers find the stored `mimetype` at a fixed offset.
            assert_eq!(&archive[30..58], b"mimetypeapplication/epub+zip");

            let mut zip = ZipArchive::new(Cursor::new(archive)).expect("A readable archive");
            let entries: Vec<(&str, u64)> = zip
                .entries()
                .iter()
                .map(|entry| (entry.name(), entry.size()))
                .collect();
            assert_eq!(
                entries,
                [
                    ("mimetype", 20),
                    ("1 - été.cbz", 200_000),
                    ("2 - empty.cbz", 0)
                ]
            );
            let compressed = zip.entries()[1].compressed_size();
            assert_eq!(compressed < 100_000, compression != Compression::none());
            for (index, expected) in [(0, &b"application/epub+zip"[..]), (1, &large)] {
                let mut content = Vec::new();
                zip.entry_reader(index)
                    .expect("Entry")
                    .read_to_end(&mut content)
                    .expect("Read the entry");
                assert!(content == expected, "entry {index} holds the written bytes");
            }
        }
    }

    #[test]
    fn dates_are_dos_fields() {
        assert_eq!(
            super::dos_date_time(date(2024, 5, 6).at(7, 8, 11, 0)),
            ((7 << 11) | (8 << 5) | 5, (44 << 9) | (5 << 5) | 6)
        );
    }
}
