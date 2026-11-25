//! The sidecars that follow an imported book.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use jiff::tz::TimeZone;
use komga_types::{FileUrl, Sidecar, SidecarKind};

use crate::library::scanning::sidecar_names;
use crate::support::file_time;

/// A book sidecar of the source and where it goes.
pub(super) struct SidecarFile {
    pub(super) source: PathBuf,
    pub(super) target: PathBuf,
}

/// The sidecar files next to the book file at `book`.
pub(super) fn sidecar_files(book: &Path) -> io::Result<Vec<PathBuf>> {
    let book_name = sidecar_names::base_name(&book.to_string_lossy()).to_owned();
    let mut files = Vec::new();
    for entry in fs::read_dir(book.parent().expect("Absolute files have parents"))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if sidecar_names::is_book_sidecar_candidate(&name)
            && sidecar_names::is_book_sidecar(&book_name, &name)
            && entry.file_type()?.is_file()
        {
            files.push(entry.path());
        }
    }
    Ok(files)
}

/// The book sidecars next to `source`, renamed after `destination_name` next to `target`.
pub(super) fn book_sidecars(
    source: &Path,
    target: &Path,
    destination_name: Option<&str>,
) -> io::Result<Vec<SidecarFile>> {
    let book_name_length = sidecar_names::base_name(&source.to_string_lossy())
        .chars()
        .count();
    let target_directory = target.parent().expect("Absolute files have parents");
    let sidecars = sidecar_files(source)?.into_iter().map(|path| {
        let name = path
            .file_name()
            .expect("Listed files have names")
            .to_string_lossy();
        let target_name = match destination_name {
            // The book name matched case-insensitively, character by character.
            Some(destination) => {
                let rest: String = name.chars().skip(book_name_length).collect();
                format!("{destination}{rest}")
            }
            None => name.into_owned(),
        };
        SidecarFile {
            target: target_directory.join(target_name),
            source: path,
        }
    });
    Ok(sidecars.collect())
}

/// Replaces any file at the target, as Kotlin does, and returns the written sidecar. Linking falls
/// back to a copy.
pub(super) fn write_sidecar(sidecar: &SidecarFile, link: bool) -> io::Result<Sidecar> {
    if link {
        match fs::remove_file(&sidecar.target) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    if !(link && fs::hard_link(&sidecar.source, &sidecar.target).is_ok()) {
        fs::copy(&sidecar.source, &sidecar.target)?;
    }
    let metadata = fs::metadata(&sidecar.target)?;
    Ok(Sidecar {
        url: FileUrl::file(&sidecar.target),
        parent_url: FileUrl::directory(
            sidecar
                .target
                .parent()
                .expect("Absolute files have parents"),
        ),
        last_modified_time: file_time::updated_time(&metadata, &TimeZone::system()),
        kind: SidecarKind::BookArtwork,
    })
}
