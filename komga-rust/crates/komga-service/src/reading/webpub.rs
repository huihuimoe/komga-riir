//! What `CommonBookController` reads for `WebPubGenerator` manifests.

use komga_db::{BookDtoDao, MediaDao, SeriesMetadataDao};
use komga_types::{
    BookDetails, BookId, BookPage, KomgaUser, MediaExtensionEpub, MediaFile, MediaProfile,
    ReadingDirection,
};

use super::{ReadingError, ReadingService};
use crate::books::analysis::pdf_extractor;
use crate::support::access_policy;

/// The book, its media, and its series' reading settings, for a manifest.
pub struct ManifestSource {
    pub book: BookDetails,
    pub profile: Option<MediaProfile>,
    /// The pages as read, PDF pages as rendered (`getPdfPagesDynamic`).
    pub pages: Vec<BookPage>,
    pub files: Vec<MediaFile>,
    /// Read for EPUB books only; `None` without a stored extension.
    pub extension: Option<MediaExtensionEpub>,
    pub series_language: String,
    pub reading_direction: Option<ReadingDirection>,
}

impl ReadingService {
    /// `getWebPubManifestInternal`'s choice of manifest; `MediaNotReady` (404 "Book analysis
    /// failed") for media types without a profile.
    pub async fn manifest_profile(&self, id: &BookId) -> Result<MediaProfile, ReadingError> {
        let mut tx = self.db.begin().await?;
        let media = MediaDao::find_by_id(&mut tx, id).await?;
        tx.rollback().await?;
        let media = media.ok_or(ReadingError::NotFound)?;
        MediaProfile::from_media_type(media.media_type.as_deref())
            .ok_or(ReadingError::MediaNotReady)
    }

    /// `getWebPubManifest{Epub,Pdf,Divina}Internal`: a `required` profile is checked before the
    /// content restrictions; DiViNa manifests require none.
    pub async fn manifest_source(
        &self,
        user: &KomgaUser,
        id: &BookId,
        required: Option<MediaProfile>,
    ) -> Result<ManifestSource, ReadingError> {
        let mut tx = self.db.begin().await?;
        let details = BookDtoDao::find_by_id(&mut tx, id, &user.id)
            .await?
            .ok_or(ReadingError::NotFound)?;
        let profile = MediaProfile::from_media_type(details.media.media_type.as_deref());
        if required.is_some_and(|required| profile != Some(required)) {
            // `MediaDto.mediaType` is empty for unanalyzed books.
            let media_type = details.media.media_type.unwrap_or_default();
            return Err(ReadingError::IncompatibleProfile(Some(media_type)));
        }
        if !access_policy::can_access_book(&mut tx, user, &details.book).await? {
            return Err(ReadingError::Restricted);
        }
        let media = MediaDao::find_by_id(&mut tx, id)
            .await?
            .ok_or_else(|| ReadingError::MissingMedia(id.clone()))?;
        let extension = match profile {
            Some(MediaProfile::Epub) => MediaDao::find_extension(&mut tx, id).await?,
            _ => None,
        };
        let (series_language, reading_direction) =
            SeriesMetadataDao::find_language_and_reading_direction(
                &mut tx,
                &details.book.series_id,
            )
            .await?;
        tx.rollback().await?;
        let pages = match profile {
            Some(MediaProfile::Pdf) => pdf_extractor::rendered_pages(media.pages),
            _ => media.pages,
        };
        Ok(ManifestSource {
            book: details,
            profile,
            pages,
            files: media.files,
            extension,
            series_language,
            reading_direction,
        })
    }
}

/// `ImageConverter.canConvertMediaType(mediaType, "image/jpeg")`.
#[must_use]
pub fn can_convert_to_jpeg(media_type: &str) -> bool {
    komga_image::can_read(media_type)
}
