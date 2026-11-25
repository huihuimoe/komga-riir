//! Analysis of `.mobi` books into virtual EPUB `Media` (spec §16.4).

use std::path::Path;

use komga_archive::mobi::{MobiError, MobiPublication};
use komga_types::{Media, MediaExtensionEpub, MediaFile, MediaStatus};
use tracing::warn;

use super::BookAnalysisError;
use super::failure::{failed, io_comment};

/// Analyzes a `.mobi` file as a virtual EPUB publication.
///
/// Unsupported variants (DRM, standalone KF8, unknown compression/encoding) are recorded as
/// [`MediaStatus::Unsupported`] (`ERR_1001`); corrupt, empty, or oversized files are recorded as
/// [`MediaStatus::Error`] (`ERR_1005`); transient OS I/O errors fail the task so it can retry.
pub(super) fn analyze(path: &Path) -> Result<Media, BookAnalysisError> {
    let publication = match MobiPublication::open(path) {
        Ok(publication) => publication,
        Err(MobiError::Unsupported(reason)) => {
            warn!(path = %path.display(), ?reason, "Unsupported MOBI variant");
            return Ok(Media::failed(
                MediaStatus::Unsupported,
                Some("ERR_1001".to_owned()),
            ));
        }
        Err(cause) => {
            let comment = match &cause {
                MobiError::Io(io) => io_comment(io),
                _ => "ERR_1005".to_owned(),
            };
            return failed(path, &cause, comment);
        }
    };

    let resources = publication.resources();
    let positions = publication.positions(&resources);
    let toc = publication.toc().to_vec();
    let page_count =
        i32::try_from(publication.page_count()).expect("A MOBI within the size limit fits i32");
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
        pages: Vec::new(),
        page_count,
        files,
        comment: None,
        epub_divina_compatible: false,
        epub_is_kepub: false,
        extension: Some(MediaExtensionEpub {
            toc,
            landmarks: Vec::new(),
            page_list: Vec::new(),
            is_fixed_layout: false,
            positions,
        }),
    })
}
