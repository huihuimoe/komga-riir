//! Filesystem probes outside database transactions, on the foreground blocking pool.

use std::fs;
use std::io::ErrorKind;
use std::path;

use komga_types::{FileUrl, Library};

use super::LibraryError;
use crate::support::blocking;

pub(super) async fn validate(
    library: Library,
    existing: Vec<Library>,
) -> Result<Library, LibraryError> {
    blocking::run(move || validate_blocking(library, &existing)).await?
}

fn validate_blocking(mut library: Library, existing: &[Library]) -> Result<Library, LibraryError> {
    // `components` drops the trailing `/` that `absolute` keeps, as `Paths.get` does.
    library.root = path::absolute(&library.root)?.components().collect();
    // Kotlin reports the URL of a path that is not a directory, which has no trailing `/`.
    let root_url = FileUrl::file(&library.root);
    let metadata = match fs::metadata(&library.root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err(LibraryError::Invalid(format!(
                "Library root folder does not exist: {root_url}"
            )));
        }
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() {
        return Err(LibraryError::Invalid(format!(
            "Library root folder is not a folder: {root_url}"
        )));
    }
    let others = || existing.iter().filter(|other| other.id != library.id);
    if others().any(|other| other.name == library.name) {
        return Err(LibraryError::Invalid(
            "Library name already exists".to_owned(),
        ));
    }
    let root = fs::canonicalize(&library.root)?;
    for other in others() {
        // An unavailable root cannot be resolved; compare it literally, as Kotlin always does.
        let (root, other_root) = match fs::canonicalize(&other.root) {
            Ok(other_root) => (root.as_path(), other_root),
            Err(_) => (library.root.as_path(), other.root.clone()),
        };
        if root.starts_with(&other_root) {
            return Err(LibraryError::Invalid(format!(
                "Library path {} is a child of existing library {}: {}",
                library.root.display(),
                other.name,
                other.root.display()
            )));
        }
        if other_root.starts_with(root) {
            return Err(LibraryError::Invalid(format!(
                "Library path {} is a parent of existing library {}: {}",
                library.root.display(),
                other.name,
                other.root.display()
            )));
        }
    }
    library.oneshots_directory = library
        .oneshots_directory
        .filter(|value| !value.trim().is_empty());
    Ok(library)
}
