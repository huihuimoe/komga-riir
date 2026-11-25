//! ZIP book files for tests, written member by member so that names, order, and compression
//! methods are exact.

use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

use komga_archive::zip::ZipArchive;

pub const STORED: u16 = 0;
pub const DEFLATED: u16 = 8;
/// commons-compress has no codec for this method, so the member cannot be read.
pub const UNSUPPORTED: u16 = 99;
/// ZSTD: Komga ships without its codec class, which fails the whole analysis.
pub const ZSTD: u16 = 93;

/// A ZIP of `(name, method, data)` members in central directory order; data is stored as given,
/// whatever the method says. Names ending in `/` are directories.
pub fn zip(members: &[(&str, u16, &[u8])]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for &(name, method, data) in members {
        let offset = u32::try_from(archive.len()).expect("Small test archive");
        let size = u32::try_from(data.len()).expect("Small test member");
        let name_length = u16::try_from(name.len()).expect("Short test name");
        let crc = crc32fast::hash(data);
        // Version 2.0, UTF-8 names (flag 11), no time.
        let common = |record: &mut Vec<u8>| {
            for value in [20_u16, 1 << 11, method, 0, 0] {
                record.extend(value.to_le_bytes());
            }
            for value in [crc, size, size] {
                record.extend(value.to_le_bytes());
            }
            record.extend(name_length.to_le_bytes());
            record.extend(0_u16.to_le_bytes());
        };
        archive.extend(0x0403_4b50_u32.to_le_bytes());
        common(&mut archive);
        archive.extend(name.as_bytes());
        archive.extend(data);

        directory.extend(0x0201_4b50_u32.to_le_bytes());
        directory.extend(20_u16.to_le_bytes());
        common(&mut directory);
        // Comment length, disk, internal and external attributes, local header offset.
        directory.extend([0; 10]);
        directory.extend(offset.to_le_bytes());
        directory.extend(name.as_bytes());
    }
    // Saturated for larger archives: readers walk the directory instead.
    let count = u16::try_from(members.len()).unwrap_or(u16::MAX);
    let directory_offset = u32::try_from(archive.len()).expect("Small test archive");
    let directory_size = u32::try_from(directory.len()).expect("Small test directory");
    archive.extend(directory);
    archive.extend(0x0605_4b50_u32.to_le_bytes());
    archive.extend([0; 4]);
    archive.extend(count.to_le_bytes());
    archive.extend(count.to_le_bytes());
    archive.extend(directory_size.to_le_bytes());
    archive.extend(directory_offset.to_le_bytes());
    archive.extend([0; 2]);
    archive
}

/// The members of `archive` with their content, in central directory order.
pub fn entries(archive: Vec<u8>) -> Vec<(String, Vec<u8>)> {
    let mut zip = ZipArchive::new(Cursor::new(archive)).expect("A readable archive");
    let names: Vec<String> = zip
        .entries()
        .iter()
        .map(|entry| entry.name().to_owned())
        .collect();
    let mut found = Vec::new();
    for (index, name) in names.into_iter().enumerate() {
        let mut content = Vec::new();
        zip.entry_reader(index)
            .expect("Entry")
            .read_to_end(&mut content)
            .expect("Entry content");
        found.push((name, content));
    }
    found
}

/// A fixture file of another workspace crate, such as `komga-image/tests/fixtures/photo.png`.
pub fn fixture(path: &str) -> Vec<u8> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    fs::read(crates.join(path)).expect("Fixture file")
}
