//! The book covers OPDS serves: Kotlin `OpdsController.getBookThumbnailSmall` and
//! `OpdsCommonController.getBookThumbnail`.

use komga_db::MediaDao;
use komga_image::{self as image, ImageType};
use komga_types::{BookId, KomgaUser, ThumbnailType};
use tokio::task;
use tracing::error;

use super::thumbnail_generator::{self, GeneratorError};
use super::{ArtworkError, ArtworkService, generator_error};
use crate::settings::ServerSettings;

impl ArtworkService {
    /// The selected thumbnail; one that was not generated at the configured size is scaled down
    /// to it as a JPEG, or served as is when that fails.
    pub async fn book_thumbnail_small(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<Vec<u8>, ArtworkError> {
        self.visible_book(user, id).await?;
        let (kind, bytes) = self
            .selected_book_thumbnail_of_kind(id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        if kind == ThumbnailType::Generated {
            return Ok(bytes);
        }
        let size = ServerSettings::load(&self.db)
            .await?
            .thumbnail_size
            .max_edge();
        let book_id = id.clone();
        let resized =
            task::spawn_blocking(move || match image::resize(&bytes, ImageType::Jpeg, size) {
                Ok(resized) => resized,
                Err(failure) => {
                    error!(book = %book_id, %failure, "Resize thumbnail failed");
                    bytes
                }
            })
            .await?;
        Ok(resized)
    }

    /// The cover at its own size as a JPEG: read from the book when the selected thumbnail was
    /// generated from it, otherwise the selected thumbnail.
    pub async fn book_poster(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<Vec<u8>, ArtworkError> {
        let book = self.visible_book(user, id).await?;
        let (kind, bytes) = self
            .selected_book_thumbnail_of_kind(id)
            .await?
            .ok_or(ArtworkError::NotFound)?;
        if kind != ThumbnailType::Generated {
            return task::spawn_blocking(move || {
                if thumbnail_generator::is_jpeg(&bytes) {
                    Ok(bytes)
                } else {
                    image::convert(&bytes, ImageType::Jpeg).map_err(ArtworkError::Image)
                }
            })
            .await?;
        }
        let mut tx = self.db.begin().await?;
        let media = MediaDao::find_by_id(&mut tx, id).await?;
        tx.rollback().await?;
        let media = media.ok_or_else(|| ArtworkError::MissingMedia(id.clone()))?;
        let path = book
            .url
            .to_path()
            .ok_or_else(|| ArtworkError::InvalidUrl(book.url.clone()))?;
        task::spawn_blocking(move || thumbnail_generator::poster(&path, &media))
            .await?
            .map_err(|failure| match failure {
                GeneratorError::NoCover => ArtworkError::NotFound,
                failure => generator_error(id, failure),
            })
    }
}
