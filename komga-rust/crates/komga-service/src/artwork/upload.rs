//! Uploaded posters, as `BookController` and `SeriesController` read them before
//! `addThumbnailForBook` and `addThumbnailForSeries`.

use komga_archive::media_type;
use komga_db::NewThumbnail;
use komga_image::read_dimension;
use komga_types::{Dimension, ThumbnailDetails, ThumbnailId, ThumbnailType};

use super::ArtworkError;
use crate::support::blocking;

/// Probes an uploaded thumbnail on the bounded blocking pool so media-type detection and image
/// header decoding never run on Tokio worker threads or under a SQLite write lock.
pub(super) async fn details(
    bytes: &[u8],
    selected: bool,
) -> Result<ThumbnailDetails, ArtworkError> {
    let owned = bytes.to_vec();
    blocking::run(move || probe_details(&owned, selected)).await?
}

/// A new `USER_UPLOADED` thumbnail: `ContentDetector` must find an image, whose
/// `ImageAnalyzer.getDimension` is 0 x 0 when unreadable.
fn probe_details(bytes: &[u8], selected: bool) -> Result<ThumbnailDetails, ArtworkError> {
    let media_type = media_type::detect(bytes)?;
    if !media_type::is_image(media_type) {
        return Err(ArtworkError::NotImage);
    }
    Ok(ThumbnailDetails {
        id: ThumbnailId::generate(),
        kind: ThumbnailType::UserUploaded,
        selected,
        media_type: media_type.to_owned(),
        file_size: i64::try_from(bytes.len()).expect("An upload fits in i64"),
        dimension: read_dimension(bytes).unwrap_or(Dimension {
            width: 0,
            height: 0,
        }),
    })
}

pub(super) fn row<'a>(details: &'a ThumbnailDetails, bytes: &'a [u8]) -> NewThumbnail<'a> {
    NewThumbnail {
        id: &details.id,
        thumbnail: Some(bytes),
        url: None,
        kind: details.kind,
        media_type: &details.media_type,
        file_size: details.file_size,
        dimension: details.dimension,
    }
}
