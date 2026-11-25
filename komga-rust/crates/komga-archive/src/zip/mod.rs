//! ZIP archives as Komga's commons-compress 1.28 `ZipFile` reads them: entry names in UTF-8
//! with `?` for malformed bytes, renamed by Info-ZIP Unicode Path fields only once local file
//! headers are read, and entry data decoded without CRC checks.
//!
//! Why not the `zip` crate: page names are stored and pages fetched by name, so a name that
//! decodes differently loses the page. `zip` decodes names without the UTF-8 flag as CP437
//! (Komga: UTF-8) and applies Unicode Path fields regardless of
//! the flag and of local headers. It also fails readable books: a Unicode Path CRC mismatch
//! fails the archive (Komga ignores the field), and a data CRC mismatch fails the entry
//! (Komga does not check). Well-formed archives read alike either way; the cost here is the
//! parsing below, which `tests` checks against commons-compress.

mod bit_reader;
mod central_directory;
mod entry_reader;
mod explode;
mod extra_field;
mod java_utf8;
mod unshrink;
pub mod writer;

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Errors from opening an archive or one of its entries.
#[derive(Debug, thiserror::Error)]
pub enum ZipError {
    /// The archive or the entry data is malformed, unsupported or unreadable.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// A ZSTD or XZ entry: Komga ships without these codecs, and the missing class fails the
    /// whole analysis instead of the one entry.
    #[error("compression method {0} needs a codec Komga does not ship")]
    MissingCodec(u16),
    /// Kotlin `EntryNotFoundException`.
    #[error("Entry does not exist: {0}")]
    EntryNotFound(String),
    /// A resource limit Komga does not have, so that a hostile archive cannot exhaust memory.
    #[error("{0}")]
    LimitExceeded(&'static str),
}

/// Largest decoded entry read whole.
const MAX_ENTRY_BYTES: usize = 256 * 1024 * 1024;

/// An archive member, in central directory order.
#[derive(Debug)]
pub struct Entry {
    name: String,
    size: u64,
    method: u16,
    flags: u16,
    compressed_size: u64,
    local_header_offset: u64,
    data_offset: Option<u64>,
}

impl Entry {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The uncompressed size the central directory records.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The compressed size the central directory records.
    #[must_use]
    pub fn compressed_size(&self) -> u64 {
        self.compressed_size
    }

    #[must_use]
    pub fn is_directory(&self) -> bool {
        self.name.ends_with('/')
    }
}

pub struct ZipArchive<R> {
    reader: R,
    entries: Vec<Entry>,
    central_directory_offset: u64,
}

impl<R: Read + Seek> ZipArchive<R> {
    /// Reads the central directory and every local file header, so that Unicode Path fields
    /// in either rename entries.
    ///
    /// # Errors
    /// [`ZipError::LimitExceeded`] for more than 100,000 entries or more than 64 MiB of names
    /// and extra fields; [`ZipError::Io`] for an unreadable or malformed archive.
    pub fn new(reader: R) -> Result<Self, ZipError> {
        Self::open(reader, /* local_headers= */ true)
    }

    /// Reads the central directory alone (`setIgnoreLocalFileHeader(true)`): names stay as
    /// stored, and an entry's local file header is read when it is opened.
    pub fn new_central_directory_only(reader: R) -> Result<Self, ZipError> {
        Self::open(reader, /* local_headers= */ false)
    }

    fn open(mut reader: R, local_headers: bool) -> Result<Self, ZipError> {
        let directory = central_directory::read(&mut reader, local_headers)?;
        Ok(Self {
            reader,
            entries: directory.entries,
            central_directory_offset: directory.offset,
        })
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The decoded data of `entries()[index]`. Read IMPLODING data in 8 KiB chunks, as Komga
    /// does, for damaged entries to end where they end in Komga.
    ///
    /// # Panics
    ///
    /// When `index` is out of bounds.
    pub fn entry_reader(&mut self, index: usize) -> Result<Box<dyn Read + '_>, ZipError> {
        let entry = &mut self.entries[index];
        let data_offset = match entry.data_offset {
            Some(offset) => offset,
            None => {
                central_directory::resolve_local_header(
                    &mut self.reader,
                    entry,
                    self.central_directory_offset,
                )?;
                entry.data_offset.expect("Resolved with the local header")
            }
        };
        self.reader.seek(SeekFrom::Start(data_offset))?;
        let data = (&mut self.reader).take(entry.compressed_size);
        entry_reader::decode(entry, data)
    }

    /// Kotlin `ZipFile.getEntryBytes`: the data of the first entry named `name`, `None` when
    /// there is none. Data decoding to more than 256 MiB is [`ZipError::LimitExceeded`].
    pub fn read_named(&mut self, name: &str) -> Result<Option<Vec<u8>>, ZipError> {
        let Some(index) = self.entries.iter().position(|entry| entry.name == name) else {
            return Ok(None);
        };
        read_in_chunks(self.entry_reader(index)?).map(Some)
    }
}

/// Kotlin `getZipEntryBytes`: the data of the first entry named `name`, looked up in the
/// central directory alone first, then with local file headers. Data decoding to more than
/// 256 MiB is [`ZipError::LimitExceeded`].
pub fn read_entry(path: &Path, name: &str) -> Result<Vec<u8>, ZipError> {
    for local_headers in [false, true] {
        let reader = BufReader::new(File::open(path)?);
        let mut archive = ZipArchive::open(reader, local_headers)?;
        if let Some(bytes) = archive.read_named(name)? {
            return Ok(bytes);
        }
    }
    Err(ZipError::EntryNotFound(name.to_owned()))
}

/// Kotlin `InputStream.readBytes`, which reads `DEFAULT_BUFFER_SIZE` bytes at a time, up to
/// `MAX_ENTRY_BYTES`.
fn read_in_chunks(mut reader: impl Read) -> Result<Vec<u8>, ZipError> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8 * 1024];
    loop {
        match reader.read(&mut chunk)? {
            0 => return Ok(bytes),
            count if bytes.len() + count > MAX_ENTRY_BYTES => {
                return Err(ZipError::LimitExceeded(
                    "Entry decodes to more than 256 MiB",
                ));
            }
            count => bytes.extend_from_slice(&chunk[..count]),
        }
    }
}

#[cfg(test)]
mod tests;
