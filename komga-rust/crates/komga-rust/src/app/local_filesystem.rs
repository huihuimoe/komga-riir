//! Startup check that the database is not on a network filesystem (Kotlin
//! `ConfigurationChecker`).
//!
//! The filesystem type is the name Java's `FileStore.type()` reports: the `/proc/mounts` type of
//! the enclosing mount on Linux and `statfs.f_fstypename` on macOS. Only names starting with
//! `cifs` or `nfs` are refused, so macOS `smbfs` and Linux `smb3` mounts pass like in the baseline.
//! Platforms without a type lookup pass as well; Java reports local volume names such as `NTFS`
//! for Windows shares.

use std::io;
use std::path::Path;

use tracing::{debug, warn};

const REMOTE_TYPES: [&str; 2] = ["cifs", "nfs"];

/// The database path is on a network filesystem.
#[derive(Debug, thiserror::Error)]
#[error(
    "The path '{path}' should be on a local filesystem, but was detected to be on a remote \
     filesystem ({store_type}). If this is inaccurate you can set '{property}: false' to ignore \
     this check."
)]
pub struct RemoteFilesystemError {
    path: String,
    store_type: String,
    property: &'static str,
}

/// Refuses a database file on a network filesystem; `property` is the setting that disables the
/// check. A missing file is judged by its parent directory; a missing parent is not checked.
///
/// # Errors
/// Returns [`RemoteFilesystemError`] when the existing path is on a `cifs*` or `nfs*` filesystem.
pub fn check_database_is_local(
    file: &Path,
    property: &'static str,
) -> Result<(), RemoteFilesystemError> {
    check_with(file, property, store_type)
}

fn check_with(
    file: &Path,
    property: &'static str,
    store_type: impl Fn(&Path) -> io::Result<String>,
) -> Result<(), RemoteFilesystemError> {
    check_if_remote(file, property, &store_type)?;
    match file.parent() {
        Some(parent) if !file.exists() => check_if_remote(parent, property, &store_type),
        _ => Ok(()),
    }
}

fn check_if_remote(
    path: &Path,
    property: &'static str,
    store_type: impl Fn(&Path) -> io::Result<String>,
) -> Result<(), RemoteFilesystemError> {
    if !path.exists() {
        return Ok(());
    }
    let store_type = store_type(path).map_or_else(
        |error| {
            warn!(path = %path.display(), %error, "Could not get FileStore type for path");
            "unknown".to_owned()
        },
        |store_type| store_type.to_lowercase(),
    );
    if REMOTE_TYPES
        .iter()
        .any(|remote| store_type.starts_with(remote))
    {
        return Err(RemoteFilesystemError {
            path: path.display().to_string(),
            store_type,
            property,
        });
    }
    debug!(store_type, path = %path.display(), "FileStore type");
    Ok(())
}

/// The type of the last-listed mount whose mount point is the longest prefix of the real path.
#[cfg(target_os = "linux")]
fn store_type(path: &Path) -> io::Result<String> {
    let path = path.canonicalize()?;
    let mounts = procfs::mounts().map_err(io::Error::other)?;
    mount_type(
        &path,
        mounts
            .into_iter()
            .map(|mount| (mount.fs_file, mount.fs_vfstype)),
    )
    .ok_or_else(|| io::Error::other("Mount point not found"))
}

#[cfg(target_os = "linux")]
fn mount_type(path: &Path, mounts: impl Iterator<Item = (String, String)>) -> Option<String> {
    let mut found: Option<(usize, String)> = None;
    for (mount_point, fs_type) in mounts {
        let depth = Path::new(&mount_point).components().count();
        if path.starts_with(&mount_point) && found.as_ref().is_none_or(|(best, _)| depth >= *best) {
            found = Some((depth, fs_type));
        }
    }
    found.map(|(_, fs_type)| fs_type)
}

#[cfg(target_os = "macos")]
fn store_type(path: &Path) -> io::Result<String> {
    let stat = rustix::fs::statfs(path)?;
    // `f_fstypename` is a NUL-terminated C string; `c_char` is signed on this target.
    let name: Vec<u8> = stat
        .f_fstypename
        .iter()
        .take_while(|&&character| character != 0)
        .map(|&character| character as u8)
        .collect();
    Ok(String::from_utf8_lossy(&name).into_owned())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn store_type(_path: &Path) -> io::Result<String> {
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const PROPERTY: &str = "komga.database.check-local-filesystem";

    #[test]
    fn remote_types_are_refused_and_others_pass() {
        let dir = komga_test_support::tempdir().expect("Failed to create directory");
        let file = dir.path().join("database.sqlite");
        fs::write(&file, b"").expect("Failed to create file");

        for remote in ["nfs", "nfs4", "NFS", "cifs", "CIFS"] {
            let error = check_with(&file, PROPERTY, |_| Ok(remote.to_owned()))
                .expect_err("Remote filesystem must be refused");
            assert_eq!(
                error.to_string(),
                format!(
                    "The path '{}' should be on a local filesystem, but was detected to be on a \
                     remote filesystem ({}). If this is inaccurate you can set \
                     'komga.database.check-local-filesystem: false' to ignore this check.",
                    file.display(),
                    remote.to_lowercase()
                )
            );
        }
        for local in ["apfs", "ext4", "smbfs", "smb3", "fuse.sshfs", ""] {
            check_with(&file, PROPERTY, |_| Ok(local.to_owned())).expect(local);
        }
        check_with(&file, PROPERTY, |_| Err(io::Error::other("statfs failed")))
            .expect("An unknown type passes");
    }

    #[test]
    fn missing_file_is_judged_by_existing_parent() {
        let dir = komga_test_support::tempdir().expect("Failed to create directory");
        let file = dir.path().join("database.sqlite");
        let error = check_with(&file, PROPERTY, |path| {
            assert_eq!(path, dir.path(), "Only the existing parent is inspected");
            Ok("nfs".to_owned())
        })
        .expect_err("A remote parent is refused");
        assert!(
            error
                .to_string()
                .contains(&dir.path().display().to_string())
        );

        let orphan = dir.path().join("missing/database.sqlite");
        check_with(&orphan, PROPERTY, |_| Ok("nfs".to_owned()))
            .expect("A missing parent is not checked");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mount_type_uses_deepest_then_last_mount() {
        let mounts = [
            ("/", "ext4"),
            ("/mnt/books", "nfs4"),
            ("/mnt/books", "cifs"),
            ("/mnt/books/local", "tmpfs"),
            ("/mnt/bookshelf", "nfs"),
        ];
        let mount_type = |path: &str| {
            mount_type(
                Path::new(path),
                mounts
                    .iter()
                    .map(|(point, fs_type)| ((*point).to_owned(), (*fs_type).to_owned())),
            )
        };
        assert_eq!(mount_type("/mnt/books/db.sqlite").as_deref(), Some("cifs"));
        assert_eq!(mount_type("/mnt/books/local/db").as_deref(), Some("tmpfs"));
        assert_eq!(mount_type("/mnt/bookshelves").as_deref(), Some("ext4"));
        assert_eq!(mount_type("/var/db").as_deref(), Some("ext4"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_reports_the_local_type() {
        let dir = komga_test_support::tempdir().expect("Failed to create directory");
        let store_type = store_type(dir.path()).expect("statfs must succeed");
        assert!(
            ["apfs", "hfs"].contains(&store_type.as_str()),
            "Unexpected type {store_type:?}"
        );
    }
}
