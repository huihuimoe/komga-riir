//! Kotlin `BookConverter.repairExtension`: a book file is renamed to the extension of its media
//! type. The renamed file is published beside the original, which is removed only once the
//! database points at the new name.

use std::fs;

use komga_types::{Book, BookId, Library};
use tokio::task;
use tracing::info;

use super::{BookConverter, ConversionError, ZIP, book_path};
use crate::library::file_operation::{PendingFile, SourceAfter};
use crate::library::scanning::scan_file;
use crate::tasks::emitter::MEDIA_TYPE_EXTENSIONS;

impl BookConverter {
    pub async fn repair_extension(&self, id: &BookId) -> Result<(), ConversionError> {
        let Some((book, extension)) = self.repairable(id).await? else {
            return Ok(());
        };
        let path = book_path(&book);
        let target = path.with_extension(extension);
        info!(source = %path.display(), target = %target.display(), "Renaming book file");
        let mut file = self
            .files
            .begin(&path, &target, None, SourceAfter::Remove)
            .await?;
        let result = self.publish_and_record(&mut file, &book).await;
        self.files.finish(file).await?;
        result
    }

    /// The book and the extension of its media type when the repair should go ahead; `None` when
    /// it is skipped.
    async fn repairable(
        &self,
        id: &BookId,
    ) -> Result<Option<(Book, &'static str)>, ConversionError> {
        let enabled = |library: &Library| library.repair_extensions;
        let Some((book, _, media)) = self.changeable(id, enabled, &self.skipped).await? else {
            return Ok(None);
        };
        let path = book_path(&book);
        let media_type = media.media_type.unwrap_or_default();
        let Some(&(_, extension)) = MEDIA_TYPE_EXTENSIONS
            .iter()
            .find(|(known, _)| *known == media_type)
        else {
            return Err(ConversionError::NotRepairable(media_type));
        };
        let actual = path
            .extension()
            .map(|actual| actual.to_string_lossy().to_lowercase());
        // An EPUB without its own media type is analyzed as a ZIP, yet still named right.
        let epub_as_zip = actual.as_deref() == Some("epub") && media_type == ZIP;
        if epub_as_zip || actual.as_deref() == Some(extension) {
            info!(book = %id, %media_type, "Extension needs no repair, skipping");
            self.skipped
                .lock()
                .expect("Not poisoned")
                .insert(id.clone());
            return Ok(None);
        }
        Ok(Some((book, extension)))
    }

    /// Writes the renamed file, publishes it and records it; on failure the caller's `finish`
    /// removes it.
    async fn publish_and_record(
        &self,
        file: &mut PendingFile,
        previous: &Book,
    ) -> Result<(), ConversionError> {
        let (source, temporary) = (file.source().to_owned(), file.temporary().to_owned());
        // The temporary file is beside the source, so a link mostly works; else copy.
        if task::spawn_blocking(move || fs::hard_link(source, temporary))
            .await?
            .is_err()
        {
            file.copy_source().await?;
        }
        self.files.publish(file).await?;
        let target = file.target().to_owned();
        let library_id = previous.library_id.clone();
        let scanned = task::spawn_blocking(move || scan_file(&target, &library_id))
            .await?
            .ok_or_else(|| ConversionError::FileNotFound(file.target().to_owned()))?;
        // The content is unchanged, so the hashes stay valid.
        let book = Book {
            name: scanned.name,
            url: scanned.url,
            file_last_modified: scanned.file_last_modified,
            file_size: scanned.file_size,
            ..previous.clone()
        };
        self.record(file, previous, &book, None, &[], &[]).await?;
        Ok(())
    }
}
