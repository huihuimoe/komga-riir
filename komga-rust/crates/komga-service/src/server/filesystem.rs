//! Authorized host filesystem browsing.
//!
//! FileSystemController browsing without modifying files or restricting admins to library roots.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use icu_casemap::CaseMapperBorrowed;
use komga_types::KomgaUser;
use thiserror::Error;

use crate::support::blocking::{self, BlockingError};

#[derive(Debug, Error)]
pub enum FilesystemError {
    #[error("Forbidden")]
    Forbidden,
    #[error("Path must be absolute")]
    RelativePath,
    #[error("Invalid filesystem path")]
    InvalidPath,
    #[error("Filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Blocking(#[from] BlockingError),
}

#[derive(Debug)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct DirectoryListing {
    pub parent: Option<PathBuf>,
    pub directories: Vec<DirectoryEntry>,
    pub files: Vec<DirectoryEntry>,
}

pub struct FilesystemService;

impl FilesystemService {
    /// Blocking filesystem operations run on the foreground pool, after checking admin access.
    pub async fn list(
        user: &KomgaUser,
        path: String,
        show_files: bool,
    ) -> Result<DirectoryListing, FilesystemError> {
        if !user.is_admin() {
            return Err(FilesystemError::Forbidden);
        }
        blocking::run(move || list_directory(path, show_files)).await?
    }
}

fn list_directory(path: String, show_files: bool) -> Result<DirectoryListing, FilesystemError> {
    if path.is_empty() {
        return Ok(DirectoryListing {
            parent: None,
            directories: roots()?,
            files: Vec::new(),
        });
    }
    // Java's getPath rejects NUL before the controller's listing-error handler.
    if path.contains('\0') {
        return Err(FilesystemError::InvalidPath);
    }
    let path = Path::new(&path);
    if !path.is_absolute() {
        return Err(FilesystemError::RelativePath);
    }
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let directory = if is_directory(path) { path } else { parent };
    let mut listing = DirectoryListing {
        parent: Some(parent.to_path_buf()),
        directories: Vec::new(),
        files: Vec::new(),
    };
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if is_hidden(&entry)? {
            continue;
        }
        let entry_path = entry.path();
        let directory = is_directory(&entry_path);
        if directory || show_files {
            let entry = DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry_path,
            };
            if directory {
                listing.directories.push(entry);
            } else {
                listing.files.push(entry);
            }
        }
    }
    // String.CASE_INSENSITIVE_ORDER uses simple upper-then-lower mappings, not full case folding.
    let mapper = CaseMapperBorrowed::new();
    let fold = |character| mapper.simple_lowercase(mapper.simple_uppercase(character));
    for entries in [&mut listing.directories, &mut listing.files] {
        entries.sort_by(|left, right| {
            left.name
                .chars()
                .map(fold)
                .cmp(right.name.chars().map(fold))
        });
    }
    Ok(listing)
}

/// Like Java's `Files.isDirectory`, any failure to read the metadata means "not a directory".
fn is_directory(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

#[cfg(not(windows))]
fn is_hidden(entry: &fs::DirEntry) -> io::Result<bool> {
    Ok(entry.file_name().as_encoded_bytes().starts_with(b"."))
}

#[cfg(windows)]
fn is_hidden(entry: &fs::DirEntry) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN;

    Ok(entry.metadata()?.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

#[cfg(not(windows))]
fn roots() -> io::Result<Vec<DirectoryEntry>> {
    Ok(vec![DirectoryEntry {
        name: "/".to_owned(),
        path: PathBuf::from("/"),
    }])
}

#[cfg(windows)]
fn roots() -> io::Result<Vec<DirectoryEntry>> {
    use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;

    // SAFETY: GetLogicalDrives has no pointer arguments or preconditions; std has no drive-list API.
    let drives = unsafe { GetLogicalDrives() };
    if drives == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut roots = Vec::with_capacity(drives.count_ones() as usize);
    for drive in 0..26 {
        if drives & (1 << drive) != 0 {
            let name = format!("{}:\\", char::from(b'A' + drive));
            roots.push(DirectoryEntry {
                path: PathBuf::from(&name),
                name,
            });
        }
    }
    Ok(roots)
}
