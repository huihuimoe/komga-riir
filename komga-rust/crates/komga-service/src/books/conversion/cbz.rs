//! The CBZ a book becomes: the members it keeps copied, then checked against the book.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use jiff::ZonedDateTime;
use komga_archive::rar::RarArchive;
use komga_archive::zip::ZipArchive;
use komga_archive::zip::writer::ZipStreamWriter;
use komga_types::{BookPage, Media, MediaStatus};

use super::{ConversionError, ZIP};

/// Kotlin `pages ∪ files` by name.
pub(super) fn member_names(media: &Media) -> HashSet<String> {
    let pages = media.pages.iter().map(|page| page.file_name.clone());
    let files = media.files.iter().map(|file| file.file_name.clone());
    pages.chain(files).collect()
}

/// Copies the `names` members of the RAR `source` into a CBZ at `target`, each once. Fails when a
/// member cannot be read or is missing.
pub(super) fn write_cbz(
    source: &Path,
    target: &Path,
    names: &HashSet<String>,
) -> Result<(), ConversionError> {
    let archive = RarArchive::open(source)?;
    let mut output = BufWriter::new(File::create(target)?);
    let mut zip = ZipStreamWriter::new(ZonedDateTime::now().datetime());
    let mut written = HashSet::new();
    let mut failure = None;
    archive.read_all(|index, data| {
        let name = archive.entries()[index].name();
        if failure.is_some() || !names.contains(name) || written.contains(name) {
            return;
        }
        let result = data.map_err(ConversionError::from).and_then(|data| {
            output.write_all(&zip.start_entry(name))?;
            output.write_all(&zip.write(&data))?;
            output.write_all(&zip.finish_entry())?;
            Ok(())
        });
        match result {
            Ok(()) => drop(written.insert(name.to_owned())),
            Err(error) => failure = Some(error),
        }
    })?;
    if let Some(error) = failure {
        return Err(error);
    }
    if written.len() != names.len() {
        return Err(ConversionError::Unverified(
            "Created file does not contain all files",
        ));
    }
    output.write_all(&zip.finish())?;
    output.flush()?;
    Ok(())
}

/// Copies the `names` members of the ZIP `source` into a CBZ at `target`, in order. Fails when a
/// member cannot be read or is missing.
pub(super) fn copy_zip(
    source: &Path,
    target: &Path,
    names: &[String],
) -> Result<(), ConversionError> {
    let mut archive = ZipArchive::new(BufReader::new(File::open(source)?))?;
    let mut output = BufWriter::new(File::create(target)?);
    let mut zip = ZipStreamWriter::new(ZonedDateTime::now().datetime());
    for name in names {
        let data = archive
            .read_named(name)?
            .ok_or(ConversionError::Unverified(
                "Created file does not contain all files",
            ))?;
        output.write_all(&zip.start_entry(name))?;
        output.write_all(&zip.write(&data))?;
        output.write_all(&zip.finish_entry())?;
    }
    output.write_all(&zip.finish())?;
    output.flush()?;
    Ok(())
}

/// Kotlin's checks: a ready ZIP holding every page, by base name and type, and every file.
pub(super) fn verify(original: &Media, converted: &Media) -> Result<(), ConversionError> {
    let pages = |media: &Media| -> HashSet<(String, String)> {
        let pages = media.pages.iter();
        pages
            .map(|page| (base_name(&page.file_name), page.media_type.clone()))
            .collect()
    };
    let files = |media: &Media| -> HashSet<String> {
        let files = media.files.iter();
        files.map(|file| base_name(&file.file_name)).collect()
    };
    if converted.status != MediaStatus::Ready {
        return Err(ConversionError::Unverified(
            "Created file could not be analyzed",
        ));
    }
    if converted.media_type.as_deref() != Some(ZIP) {
        return Err(ConversionError::Unverified(
            "Created file is not a zip file",
        ));
    }
    if !pages(original).is_subset(&pages(converted)) {
        return Err(ConversionError::Unverified(
            "Created file does not contain all pages",
        ));
    }
    if !files(original).is_subset(&files(converted)) {
        return Err(ConversionError::Unverified(
            "Created file does not contain all files",
        ));
    }
    Ok(())
}

/// Commons IO `FilenameUtils.getName`.
fn base_name(name: &str) -> String {
    name.rsplit(['/', '\\']).next().unwrap_or(name).to_owned()
}

/// Kotlin `restoreHashFrom`: a page keeps the hash of the same named page of the same size and
/// type.
pub(super) fn restore_hashes(pages: Vec<BookPage>, from: &[BookPage]) -> Vec<BookPage> {
    pages
        .into_iter()
        .map(|page| {
            let hash = from.iter().find(|previous| {
                (
                    &previous.file_size,
                    &previous.media_type,
                    &previous.file_name,
                ) == (&page.file_size, &page.media_type, &page.file_name)
                    && !previous.file_hash.trim().is_empty()
            });
            match hash {
                Some(previous) => BookPage {
                    file_hash: previous.file_hash.clone(),
                    ..page
                },
                None => page,
            }
        })
        .collect()
}
