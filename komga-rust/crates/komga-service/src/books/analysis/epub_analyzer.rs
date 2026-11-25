//! Kotlin `BookAnalyzer.analyzeEpub`: the resources, navigation, DiViNa pages, and positions of
//! an EPUB book file.

use std::io::Read;
use std::path::Path;

use komga_archive::epub::{EpubError, EpubPackage};
use komga_archive::zip::ZipError;
use komga_types::{Media, MediaExtensionEpub, MediaFile, MediaStatus};
use tracing::error;

use super::BookAnalysisError;
use super::failure::{failed, io_comment, transient};

/// A ready media unless the package cannot be opened or its hrefs are malformed, which Kotlin's
/// `analyze` records as an I/O failure or `ERR_1005`. Each other failing part is left empty and
/// noted in the comment.
///
/// # Errors
/// [`BookAnalysisError::MissingCodec`], which fails Kotlin's whole analysis, or a
/// [`transient`] read failure.
pub(super) fn analyze(
    path: &Path,
    analyze_dimensions: bool,
    letter_count_threshold: i32,
) -> Result<Media, BookAnalysisError> {
    match analyze_package(path, analyze_dimensions, letter_count_threshold) {
        Ok(media) => Ok(media),
        Err(EpubError::Zip(ZipError::MissingCodec(method))) => {
            Err(BookAnalysisError::MissingCodec(method))
        }
        Err(cause) => {
            let comment = match &cause {
                EpubError::Zip(ZipError::Io(cause)) => io_comment(cause),
                _ => "ERR_1005".to_owned(),
            };
            failed(path, &cause, comment)
        }
    }
}

fn analyze_package(
    path: &Path,
    analyze_dimensions: bool,
    letter_count_threshold: i32,
) -> Result<Media, EpubError> {
    let mut epub = EpubPackage::open(path)?;
    let (resources, missing): (Vec<_>, Vec<_>) = epub
        .resources()?
        .into_iter()
        .partition(|resource| resource.file_size.is_some());
    let is_kepub = epub.is_kepub(&resources);

    let mut errors: Vec<String> = Vec::new();
    let toc = recover(epub.toc(), "ERR_1035", &mut errors)?;
    let landmarks = recover(epub.landmarks(), "ERR_1036", &mut errors)?;
    let page_list = recover(epub.page_list(), "ERR_1037", &mut errors)?;
    let read_dimension = |reader: &mut dyn Read| komga_image::read_dimension(reader);
    let divina_pages = recover(
        epub.divina_pages(
            letter_count_threshold,
            analyze_dimensions.then_some(&read_dimension),
        ),
        "ERR_1038",
        &mut errors,
    )?;
    let is_fixed_layout = !divina_pages.is_empty() || epub.is_fixed_layout();
    let positions = recover(
        epub.positions(&resources, is_fixed_layout, is_kepub)
            .map_err(EpubError::from),
        "ERR_1039",
        &mut errors,
    )?;
    if !missing.is_empty() {
        let names: Vec<&str> = missing
            .iter()
            .map(|resource| resource.file_name.as_str())
            .collect();
        errors.push(format!("ERR_1033 [{}]", names.join(", ")));
    }

    let page_count = if divina_pages.is_empty() {
        i32::try_from(epub.page_count()).expect("A book file has fewer than 2^31 KiB")
    } else {
        i32::try_from(divina_pages.len()).expect("A book file has fewer than 2^31 members")
    };
    let files = resources
        .into_iter()
        .map(|resource| MediaFile {
            file_name: resource.file_name,
            media_type: Some(resource.media_type),
            sub_type: Some(resource.sub_type),
            file_size: resource.file_size.and_then(|size| i64::try_from(size).ok()),
        })
        .collect();
    Ok(Media {
        status: MediaStatus::Ready,
        media_type: None,
        epub_divina_compatible: !divina_pages.is_empty(),
        pages: divina_pages,
        page_count,
        files,
        comment: (!errors.is_empty()).then(|| errors.join(" ")),
        epub_is_kepub: is_kepub,
        extension: Some(MediaExtensionEpub {
            toc,
            landmarks,
            page_list,
            is_fixed_layout,
            positions,
        }),
    })
}

/// Kotlin's `catch (e: Exception)` around a part: the part is empty and `code` noted. A missing
/// codec is a `NoClassDefFoundError`, which no such clause catches; a [`transient`] read failure
/// fails the analysis.
fn recover<T: Default>(
    result: Result<T, EpubError>,
    code: &str,
    errors: &mut Vec<String>,
) -> Result<T, EpubError> {
    match result {
        Ok(value) => Ok(value),
        Err(error @ EpubError::Zip(ZipError::MissingCodec(_))) => Err(error),
        Err(error) if transient(&error).is_some() => Err(error),
        Err(error) => {
            error!(%error, code, "Error while analyzing EPUB");
            errors.push(code.to_owned());
            Ok(T::default())
        }
    }
}
