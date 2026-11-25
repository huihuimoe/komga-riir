//! RAR archives as Komga's junrar 8.1.2 reads them: members in archive order under the names
//! junrar's `FileHeader.getFileName` gives, which keep the stored `\` of Windows paths.
//!
//! Why `rars` and not libunrar bindings: page names are stored and pages fetched by name, and
//! libunrar rewrites `\` to `/` on Unix, so pages of Windows-made RAR4 books would be lost.

use std::cell::RefCell;
use std::fs::File;
use std::io::{self, Write};
use std::mem;
use std::path::Path;
use std::rc::Rc;
use std::str;

use rars::{
    Archive, ArchiveMember, ArchiveReadOptions, ArchiveReader, ExtractionDecision,
    ExtractionErrorAction,
};

/// Limits on the headers held in memory until the archive is closed, as for ZIP's central
/// directory.
const MAX_HEADERS: u64 = 100_000;
const MAX_HEADER_BYTES: u64 = 64 * 1024 * 1024;
/// Largest decoded member, as for ZIP.
const MAX_MEMBER_BYTES: u64 = 256 * 1024 * 1024;
/// The largest dictionary RAR 5 archivers before RAR 7 write.
const MAX_DICTIONARY_BYTES: u64 = 1024 * 1024 * 1024;
/// Decoder memory per decoding call, mostly the history window and the member being decoded.
const MAX_WORKSPACE_BYTES: u64 = 1024 * 1024 * 1024;

/// Errors from opening an archive or reading one of its members.
#[derive(Debug, thiserror::Error)]
pub enum RarError {
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The archive or the member data is malformed, unsupported or unreadable.
    #[error(transparent)]
    Archive(#[from] rars::Error),
    #[error("Entry does not exist: {0}")]
    EntryNotFound(String),
    #[error("RAR 1.4 archive after a `Rar!` prefix")]
    Rar14AfterPrefix,
    /// Encrypted headers or members, which Komga reads without a password.
    #[error("Encrypted RAR archives are not supported")]
    Encrypted,
    #[error("Multi-Volume RAR archives are not supported")]
    MultiVolume,
    /// A resource limit Komga does not have, so that a hostile archive cannot exhaust memory.
    #[error("Entry of {0} bytes is larger than {MAX_MEMBER_BYTES} bytes")]
    EntryTooLarge(u64),
}

/// An archive member, in archive order.
#[derive(Debug)]
pub struct Entry {
    name: String,
    size: u64,
    is_directory: bool,
}

impl Entry {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The unpacked size the header records.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    #[must_use]
    pub fn is_directory(&self) -> bool {
        self.is_directory
    }
}

pub struct RarArchive {
    archive: Archive,
    entries: Vec<Entry>,
}

impl RarArchive {
    /// Reads the member headers; member data is read on demand.
    ///
    /// # Errors
    /// The file cannot be read or is not a readable RAR archive.
    pub fn open(path: &Path) -> Result<Self, RarError> {
        // Opened first so that a missing file stays an `io::Error`. The path, not this file, is
        // parsed: rars 0.12.3 `read_reader` misplaces the end of a RAR 5 archive after a prefix.
        File::open(path)?;
        let options = ArchiveReadOptions::new()
            .with_max_header_count(MAX_HEADERS)
            .with_max_header_bytes(MAX_HEADER_BYTES);
        let archive = match ArchiveReader::read_path_with_options(path, options) {
            // `rars` is built without decryption, so encrypted headers stop the parse this way.
            Err(rars::Error::FeatureDisabled {
                feature: "encryption",
            }) => return Err(RarError::Encrypted),
            archive => archive?,
        };
        let (volume, names): (bool, Vec<String>) = match &archive {
            Archive::Rar15To40(rar4) => (
                rar4.main.is_volume(),
                archive
                    .members()
                    .map(|member| narrow_name(&member.meta.name))
                    .collect(),
            ),
            // junrar `Rar5FileHeaderReader`: the UTF-8 name, with malformed bytes replaced, and
            // `;N` for a file version `N` other than 0, which `rars` appends.
            Archive::Rar50Plus(rar5) => (
                rar5.main.is_volume(),
                rar5.files()
                    .map(|header| String::from_utf8_lossy(&header.name).into_owned())
                    .collect(),
            ),
            // Detection only lets through files starting with `Rar!`, so `rars` found this one
            // past such a prefix, where junrar finds no archive.
            Archive::Rar13(_) | _ => return Err(RarError::Rar14AfterPrefix),
        };
        // junrar `isPasswordProtected` before `isMultiVolume`. junrar reads a RAR 5 volume set
        // across its files; it is refused like a RAR 4 one.
        if archive.members().any(|member| member.meta.is_encrypted) {
            return Err(RarError::Encrypted);
        }
        if volume {
            return Err(RarError::MultiVolume);
        }
        let entries = archive
            .members()
            .zip(names)
            .map(|(member, name)| Entry {
                name,
                size: member.meta.unpacked_size,
                is_directory: member.meta.is_directory,
            })
            .collect();
        Ok(Self { archive, entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Decodes the file members in one pass, handing `visit` each member's index in
    /// [`entries`](Self::entries) with its data, or why it could not be read. A solid archive
    /// shares one decoder across members, so reading them one by one would decode each
    /// predecessor again. One member's data is held at a time.
    ///
    /// # Errors
    /// A failure that ends the pass, such as damage inside a solid stream; the member being read
    /// was visited with it, later members are not visited.
    pub fn read_all(
        &self,
        mut visit: impl FnMut(usize, Result<Vec<u8>, RarError>),
    ) -> Result<(), RarError> {
        let pass = RefCell::new(Pass {
            visit: &mut visit,
            reading: None,
            next: 0,
            data: Rc::default(),
        });
        let result = self.archive.extract_with_control_and_errors(
            extraction_options(),
            |member| Ok(pass.borrow_mut().start(member)),
            |_, error| {
                pass.borrow_mut().fail(error);
                Ok(ExtractionErrorAction::Continue)
            },
        );
        let mut pass = pass.into_inner();
        match result {
            Ok(_) => {
                pass.finish();
                Ok(())
            }
            Err(error) => {
                pass.fail(&error);
                Err(error.into())
            }
        }
    }
}

/// The state of [`RarArchive::read_all`]: `rars` reports a member's end by moving to the next.
struct Pass<'v, V> {
    visit: &'v mut V,
    reading: Option<usize>,
    next: usize,
    data: Rc<RefCell<Vec<u8>>>,
}

impl<V: FnMut(usize, Result<Vec<u8>, RarError>)> Pass<'_, V> {
    fn start(&mut self, member: &ArchiveMember) -> ExtractionDecision {
        self.finish();
        let index = self.next;
        self.next += 1;
        if member.meta.is_directory || member.meta.is_redirection {
            return ExtractionDecision::Skip;
        }
        // `rars` refuses such a member by ending the pass; skipping it lets later members of an
        // archive that is not solid be read.
        if member.meta.unpacked_size > MAX_MEMBER_BYTES {
            (self.visit)(
                index,
                Err(RarError::EntryTooLarge(member.meta.unpacked_size)),
            );
            return ExtractionDecision::Skip;
        }
        self.reading = Some(index);
        ExtractionDecision::Extract(Box::new(SharedBuffer(Rc::clone(&self.data))))
    }

    fn finish(&mut self) {
        if let Some(index) = self.reading.take() {
            (self.visit)(index, Ok(mem::take(&mut *self.data.borrow_mut())));
        }
    }

    fn fail(&mut self, error: &rars::Error) {
        if let Some(index) = self.reading.take() {
            self.data.borrow_mut().clear();
            (self.visit)(index, Err(RarError::Archive(error.clone())));
        }
    }
}

struct SharedBuffer(Rc<RefCell<Vec<u8>>>);

impl Write for SharedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Kotlin `getEntryStream`: the data of the first member named `name`; a directory has none.
///
/// # Errors
/// The archive cannot be read, has no such member, or the member cannot be decoded.
pub fn read_entry(path: &Path, name: &str) -> Result<Vec<u8>, RarError> {
    let archive = RarArchive::open(path)?;
    let index = archive
        .entries
        .iter()
        .position(|entry| entry.name == name)
        .ok_or_else(|| RarError::EntryNotFound(name.to_owned()))?;
    let data = archive
        .archive
        .read_member_at_with_options(index, extraction_options())?;
    Ok(data.unwrap_or_default())
}

/// The decoding limits; a member over them fails, and a solid pass stops at the workspace limit.
fn extraction_options() -> ArchiveReadOptions<'static> {
    ArchiveReadOptions::new()
        .with_max_member_output_bytes(MAX_MEMBER_BYTES)
        .with_rar50_dictionary_size_limit(MAX_DICTIONARY_BYTES)
        .with_max_reader_workspace_bytes(MAX_WORKSPACE_BYTES)
}

/// junrar `decodeNarrowName` for RAR 1.5–4.x: ASCII, else UTF-8, else ISO-8859-1. `rars` gives the
/// decoded Unicode name as UTF-8, or the stored bytes when the header has none or it does not
/// decode.
fn narrow_name(bytes: &[u8]) -> String {
    match str::from_utf8(bytes) {
        Ok(name) => name.to_owned(),
        Err(_) => bytes.iter().copied().map(char::from).collect(),
    }
}

#[cfg(test)]
mod tests;
