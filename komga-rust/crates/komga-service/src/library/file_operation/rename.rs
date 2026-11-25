//! Publishing a file by renaming it within its filesystem.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

/// Moves `from` to `to`, replacing a file at `to` only when `replace`, and makes the move
/// durable.
pub(super) fn rename(from: &Path, to: &Path, replace: bool) -> io::Result<()> {
    if replace {
        fs::rename(from, to)?;
    } else {
        rename_new(from, to)?;
    }
    sync_directory(to)
}

fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    match rename_exclusive(from, to) {
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::Unsupported | ErrorKind::InvalidInput
            ) =>
        {
            // Mounts such as SMB and NFS may not support an exclusive rename. Checking first, as
            // Kotlin `moveTo` does, misses a file created in between.
            if fs::exists(to)? {
                Err(ErrorKind::AlreadyExists.into())
            } else {
                fs::rename(from, to)
            }
        }
        result => result,
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn rename_exclusive(from: &Path, to: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    Ok(renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE)?)
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
fn rename_exclusive(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(ErrorKind::Unsupported.into())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path.parent().expect("A file path has a parent"))?.sync_all()
}

/// Windows has no directory handle to flush through the standard library; NTFS journals renames.
#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}
