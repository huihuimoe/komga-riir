//! `SeriesController.downloadSeriesAsZip`: the files of a series as one ZIP archive.

use komga_db::{BookDao, SeriesMetadataDao};
use komga_types::{KomgaUser, SeriesId, UserRole};

use super::{ReadingError, ReadingService};
use crate::BookArchive;
use crate::support::access_policy::{self, Access};

impl ReadingService {
    /// The archive of every book of series `id`, soft-deleted ones included, for a user with
    /// `FILE_DOWNLOAD` who may see the series. Entries are the file names; missing files are
    /// skipped. Kotlin fails on an unknown series when naming the archive, which is a 404 here.
    pub async fn series_archive(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
    ) -> Result<BookArchive, ReadingError> {
        if !user.has_role(UserRole::FileDownload) {
            return Err(ReadingError::Forbidden);
        }
        let mut tx = self.db.begin().await?;
        match access_policy::check_series_id(&mut tx, user, id).await? {
            Access::Allowed => {}
            Access::Denied => return Err(ReadingError::Restricted),
            Access::Unknown => return Err(ReadingError::NotFound),
        }
        let metadata = SeriesMetadataDao::find_by_id(&mut tx, id)
            .await?
            .ok_or(ReadingError::NotFound)?;
        let books = BookDao::find_all_by_series(&mut tx, id).await?;
        tx.rollback().await?;
        // A URL without a path names no file, like a missing one.
        let entries = books
            .into_iter()
            .filter_map(|book| book.url.to_path())
            .map(|path| {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                (file_name.into_owned(), path)
            })
            .collect();
        Ok(BookArchive::spawn(
            format!("{}.zip", metadata.title),
            entries,
        ))
    }
}
