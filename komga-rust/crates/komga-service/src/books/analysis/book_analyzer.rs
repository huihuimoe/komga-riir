//! Kotlin `BookAnalyzer.analyze`: the media of a book file.

use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;

use komga_archive::mobi::MOBI_MEDIA_TYPE;
use komga_archive::pdf::PdfError;
use komga_archive::rar::RarError;
use komga_archive::zip::ZipError;
use komga_archive::{epub, media_type};
use komga_types::{Book, BookPage, Media, MediaFile, MediaStatus};
use tracing::{info, warn};

use super::failure::{failed, io_comment};
use super::{BookAnalysisError, ContainerEntry};
use super::{epub_analyzer, mobi_analyzer, pdf_extractor, rar_extractor, zip_extractor};
use crate::support::file_time;

const ZIP: &str = "application/zip";
const RAR: [&str; 3] = [
    "application/x-rar-compressed",
    "application/x-rar-compressed; version=4",
    "application/x-rar-compressed; version=5",
];
const EPUB: &str = "application/epub+zip";
const PDF: &str = "application/pdf";

/// [`analyze`] of the file version the scan recorded for `book`, or `None` when the file is
/// another version before or after it is read (the result would describe no recorded
/// version). A file without metadata is analyzed, so its failure is stored as in Kotlin.
pub(super) fn analyze_recorded(
    path: &Path,
    book: &Book,
    analyze_dimensions: bool,
    epub_letter_count_threshold: i32,
) -> Result<Option<Media>, BookAnalysisError> {
    let recorded = || {
        fs::metadata(path)
            .ok()
            .map(|metadata| file_time::is_recorded(&metadata, book))
    };
    let before = recorded();
    if before == Some(false) {
        return Ok(None);
    }
    let media = analyze(path, analyze_dimensions, epub_letter_count_threshold)?;
    Ok((recorded() == before).then_some(media))
}

/// # Errors
/// [`BookAnalysisError::MissingCodec`]: Kotlin's `NoClassDefFoundError` escapes every
/// `catch (Exception)`, so the task fails and the stored media stays. [`BookAnalysisError::Io`]:
/// see [`transient`].
pub(crate) fn analyze(
    path: &Path,
    analyze_dimensions: bool,
    epub_letter_count_threshold: i32,
) -> Result<Media, BookAnalysisError> {
    info!(path = %path.display(), "Trying to analyze book");
    let detected = File::open(path).and_then(|file| media_type::detect(BufReader::new(file)));
    let mut media_type = match detected {
        Ok(media_type) => media_type,
        Err(cause) => return failed(path, &cause, io_comment(&cause)),
    };
    info!(media_type, "Detected media type");
    if ![ZIP, EPUB, PDF, MOBI_MEDIA_TYPE].contains(&media_type) && !RAR.contains(&media_type) {
        return Ok(Media {
            media_type: Some(media_type.to_owned()),
            ..Media::failed(MediaStatus::Unsupported, Some("ERR_1001".to_owned()))
        });
    }
    if kotlin_extension(path).eq_ignore_ascii_case("epub") && media_type != EPUB {
        if epub::is_epub(path) {
            media_type = EPUB;
        } else {
            warn!(path = %path.display(), "Epub file is malformed, file is probably broken");
            return Ok(Media {
                media_type: Some(media_type.to_owned()),
                ..Media::failed(MediaStatus::Error, Some("ERR_1032".to_owned()))
            });
        }
    }
    let media = match media_type {
        ZIP => analyze_zip(path, analyze_dimensions)?,
        EPUB => epub_analyzer::analyze(path, analyze_dimensions, epub_letter_count_threshold)?,
        MOBI_MEDIA_TYPE => mobi_analyzer::analyze(path)?,
        PDF => analyze_pdf(path, analyze_dimensions)?,
        // The `RAR` types; the check above lets nothing else through.
        _ => analyze_rar(path, analyze_dimensions)?,
    };
    Ok(Media {
        media_type: Some(media_type.to_owned()),
        ..media
    })
}

/// Kotlin `Path.extension`: the file name after its last `.`.
fn kotlin_extension(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    name.rsplit_once('.')
        .map(|(_, extension)| extension.to_owned())
        .unwrap_or_default()
}

/// Kotlin `analyzeDivina` with `ZipExtractor`.
fn analyze_zip(path: &Path, analyze_dimensions: bool) -> Result<Media, BookAnalysisError> {
    match zip_extractor::entries(path, analyze_dimensions) {
        Ok(entries) => Ok(divina_media(path, entries)),
        Err(ZipError::MissingCodec(method)) => Err(BookAnalysisError::MissingCodec(method)),
        Err(cause) => failed(path, &cause, "ERR_1008".to_owned()),
    }
}

/// Kotlin `analyzeDivina` with `RarExtractor`, whose `MediaUnsupportedException` codes are
/// stored without logging.
fn analyze_rar(path: &Path, analyze_dimensions: bool) -> Result<Media, BookAnalysisError> {
    let unsupported = |code: &str| Media::failed(MediaStatus::Unsupported, Some(code.to_owned()));
    match rar_extractor::entries(path, analyze_dimensions) {
        Ok(entries) => Ok(divina_media(path, entries)),
        Err(RarError::Encrypted) => Ok(unsupported("ERR_1002")),
        Err(RarError::MultiVolume) => Ok(unsupported("ERR_1004")),
        Err(cause) => failed(path, &cause, "ERR_1008".to_owned()),
    }
}

/// Kotlin `analyzePdf`, whose exceptions reach the `analyze` catch clauses. Unlike archives, a
/// document without pages is ready.
fn analyze_pdf(path: &Path, analyze_dimensions: bool) -> Result<Media, BookAnalysisError> {
    match pdf_extractor::pages(path, analyze_dimensions) {
        Ok(pages) => {
            let page_count = i32::try_from(pages.len())
                .expect("A PDF within the file size limit has fewer than 2^31 pages");
            Ok(Media {
                status: MediaStatus::Ready,
                media_type: None,
                pages,
                page_count,
                files: Vec::new(),
                comment: None,
                epub_divina_compatible: false,
                epub_is_kepub: false,
                extension: None,
            })
        }
        Err(cause) => {
            let comment = match &cause {
                PdfError::Io(cause) => io_comment(cause),
                _ => "ERR_1005".to_owned(),
            };
            failed(path, &cause, comment)
        }
    }
}

/// Images are pages, other members are files, and members that could not be read are listed in
/// an `ERR_1007` comment.
fn divina_media(path: &Path, entries: Vec<ContainerEntry>) -> Media {
    let mut pages = Vec::new();
    let mut files = Vec::new();
    let mut unreadable = Vec::new();
    for entry in entries {
        match entry.media_type {
            Some(media_type) if media_type::is_image(media_type) => pages.push(BookPage {
                file_name: entry.name,
                media_type: media_type.to_owned(),
                dimension: entry.dimension,
                file_hash: String::new(),
                file_size: entry.file_size,
            }),
            media_type => {
                if media_type.is_none() {
                    unreadable.push(entry.name.clone());
                }
                files.push(MediaFile {
                    file_name: entry.name,
                    media_type: media_type.map(str::to_owned),
                    sub_type: None,
                    file_size: entry.file_size,
                });
            }
        }
    }
    if pages.is_empty() {
        warn!(path = %path.display(), "Book does not contain any pages");
        return Media::failed(MediaStatus::Error, Some("ERR_1006".to_owned()));
    }
    info!(pages = pages.len(), "Book has pages");
    let page_count =
        i32::try_from(pages.len()).expect("Readable archives have fewer than 2^31 members");
    Media {
        status: MediaStatus::Ready,
        media_type: None,
        pages,
        page_count,
        files,
        comment: (!unreadable.is_empty()).then(|| format!("ERR_1007 [{}]", unreadable.join(", "))),
        epub_divina_compatible: false,
        epub_is_kepub: false,
        extension: None,
    }
}
