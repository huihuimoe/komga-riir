use std::fs::{self, File};
use std::io::{self, BufReader, Cursor, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use super::{MAX_ENTRY_BYTES, ZipArchive, ZipError, read_entry, read_in_chunks};

fn digest(reader: impl Read) -> String {
    match read_in_chunks(reader) {
        Ok(bytes) => hex::encode(Sha256::digest(&bytes))[..16].to_owned(),
        Err(_) => "ERR".to_owned(),
    }
}

fn failure(error: &ZipError) -> &'static str {
    match error {
        ZipError::MissingCodec(_) => "FATAL",
        ZipError::EntryNotFound(_) => "NOTFOUND",
        ZipError::Io(_) => "ERR",
        ZipError::LimitExceeded(_) => "LIMIT",
    }
}

/// What Komga sees in `archive`, in the lines of `expected.tsv`.
fn observe(directory: &Path, archive: &str) -> Vec<String> {
    let path = directory.join(archive);
    let file = BufReader::new(File::open(&path).expect("Fixture"));
    let mut zip = match ZipArchive::new(file) {
        Ok(zip) => zip,
        Err(_) => return vec![format!("{archive}\tOPEN\tERR")],
    };
    let mut lines = Vec::new();
    let mut names = Vec::new();
    for index in 0..zip.entries().len() {
        let entry = &zip.entries()[index];
        let (name, size, directory) = (entry.name().to_owned(), entry.size(), entry.is_directory());
        let content = if directory {
            "-".to_owned()
        } else {
            names.push(name.clone());
            zip.entry_reader(index)
                .map_or_else(|error| failure(&error).to_owned(), digest)
        };
        let name = name.replace('\\', "\\\\");
        lines.push(format!(
            "{archive}\tENTRY\t{name}\t{size}\t{directory}\t{content}"
        ));
    }
    for name in names {
        let content = read_entry(&path, &name).map_or_else(
            |error| failure(&error).to_owned(),
            |bytes| digest(&bytes[..]),
        );
        lines.push(format!(
            "{archive}\tBYNAME\t{}\t{content}",
            name.replace('\\', "\\\\")
        ));
    }
    lines
}

/// `expected.tsv` holds what commons-compress 1.28 reports for each fixture through Komga's
/// `ZipExtractor` and `getZipEntryBytes` calls: the entries in central directory order with
/// name, size, whether a directory, and content digest or failure; then every file entry read
/// by name. The fixtures are built byte by byte to cover names, extra fields, methods and
/// malformed layouts; the `pkzip-*` ones hold PKZIP 1.1 data from the commons-compress test
/// resources (Apache-2.0).
#[test]
fn reading_matches_commons_compress() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zip");
    let expected = fs::read_to_string(directory.join("expected.tsv")).expect("Expected results");
    let mut archives: Vec<String> = fs::read_dir(&directory)
        .expect("Fixtures")
        .map(|entry| {
            entry
                .expect("Fixture")
                .file_name()
                .into_string()
                .expect("ASCII name")
        })
        .filter(|name| name.ends_with(".zip"))
        .collect();
    archives.sort();
    let observed: Vec<String> = archives
        .iter()
        .flat_map(|archive| observe(&directory, archive))
        .collect();
    let expected: Vec<&str> = expected.lines().collect();
    assert_eq!(observed, expected);
}

/// An archive of empty stored entries, each `(name, extra field length)`. The end record's
/// counts saturate: readers walk the directory instead.
fn empty_entries(entries: &[(&str, u16)]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for &(name, extra_length) in entries {
        let offset = u32::try_from(archive.len()).expect("Test archives stay under 4 GiB");
        let name_length = u16::try_from(name.len()).expect("Short test name");
        // Version 2.0, UTF-8 names, stored, no time, CRC and sizes 0.
        let common = |record: &mut Vec<u8>, extra_length: u16| {
            for value in [20_u16, 1 << 11, 0, 0, 0] {
                record.extend(value.to_le_bytes());
            }
            record.extend([0; 12]);
            record.extend(name_length.to_le_bytes());
            record.extend(extra_length.to_le_bytes());
        };
        archive.extend(b"PK\x03\x04");
        common(&mut archive, 0);
        archive.extend(name.as_bytes());
        directory.extend(b"PK\x01\x02");
        directory.extend(20_u16.to_le_bytes());
        common(&mut directory, extra_length);
        directory.extend([0; 10]);
        directory.extend(offset.to_le_bytes());
        directory.extend(name.as_bytes());
        if extra_length > 0 {
            // One field of an unknown kind filling the whole length.
            directory.extend(0xcafe_u16.to_le_bytes());
            directory.extend((extra_length - 4).to_le_bytes());
            directory.resize(directory.len() + usize::from(extra_length - 4), 0);
        }
    }
    let count = u16::try_from(entries.len()).unwrap_or(u16::MAX);
    let directory_offset = u32::try_from(archive.len()).expect("Test archives stay under 4 GiB");
    let directory_size = u32::try_from(directory.len()).expect("Test archives stay under 4 GiB");
    archive.extend(directory);
    archive.extend(b"PK\x05\x06");
    archive.extend([0; 4]);
    archive.extend(count.to_le_bytes());
    archive.extend(count.to_le_bytes());
    archive.extend(directory_size.to_le_bytes());
    archive.extend(directory_offset.to_le_bytes());
    archive.extend([0; 2]);
    archive
}

fn open(archive: Vec<u8>) -> Result<usize, ZipError> {
    ZipArchive::new(Cursor::new(archive)).map(|zip| zip.entries().len())
}

#[test]
fn central_directories_hold_at_most_100000_entries() {
    let names: Vec<String> = (0..=100_000).map(|index| index.to_string()).collect();
    let entries: Vec<(&str, u16)> = names.iter().map(|name| (name.as_str(), 0)).collect();
    // Far larger than the 64 KiB tail searched for the end record.
    assert_eq!(open(empty_entries(&entries[..100_000])).ok(), Some(100_000));
    assert!(matches!(
        open(empty_entries(&entries)),
        Err(ZipError::LimitExceeded(
            "Archive lists more than 100000 entries"
        ))
    ));
}

#[test]
fn central_directories_hold_at_most_64_mib_of_names_and_extra_fields() {
    // 1024 entries of 65,536 name and extra field bytes fill 64 MiB exactly.
    let mut entries = vec![("a", u16::MAX); 1024];
    assert_eq!(open(empty_entries(&entries)).ok(), Some(1024));
    entries.push(("b", 0));
    assert!(matches!(
        open(empty_entries(&entries)),
        Err(ZipError::LimitExceeded(
            "Archive names and extra fields exceed 64 MiB"
        ))
    ));
}

#[test]
fn entries_read_whole_decode_to_at_most_256_mib() {
    let limit = u64::try_from(MAX_ENTRY_BYTES).expect("256 MiB");
    let whole = read_in_chunks(io::repeat(0).take(limit)).expect("At the limit");
    assert_eq!(whole.len(), MAX_ENTRY_BYTES);
    assert!(matches!(
        read_in_chunks(io::repeat(0).take(limit + 1)),
        Err(ZipError::LimitExceeded(
            "Entry decodes to more than 256 MiB"
        ))
    ));
}
