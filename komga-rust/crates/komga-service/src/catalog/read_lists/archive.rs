//! `ReadListController.downloadReadListAsZip`: the files of a read list as one ZIP archive.

use komga_db::ReadListDao;
use komga_types::{ContentScope, KomgaUser, ReadListId, UserRole};

use super::{ReadListError, ReadListService};
use crate::support::book_archive::BookArchive;

impl ReadListService {
    /// The archive of the books of read list `id` that `user`, who needs `FILE_DOWNLOAD`, sees.
    /// Entries are named `<number + 1> - <file name>`; books whose file is missing are skipped.
    pub async fn archive(
        &self,
        user: &KomgaUser,
        id: &ReadListId,
    ) -> Result<BookArchive, ReadListError> {
        if !user.has_role(UserRole::FileDownload) {
            return Err(ReadListError::Forbidden);
        }
        let mut tx = self.db.begin().await?;
        let scope = ContentScope::for_user(user);
        let read_list = ReadListDao::find_by_id(&mut tx, id, Some(&scope))
            .await?
            .ok_or(ReadListError::NotFound)?;
        let urls = ReadListDao::find_book_urls(&mut tx, id, &scope).await?;
        tx.commit().await?;
        // A URL without a path names no file, like a missing one.
        let entries = urls
            .into_iter()
            .filter_map(|(number, url)| url.to_path().map(|path| (number, path)))
            .map(|(number, path)| {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                (format!("{} - {file_name}", i64::from(number) + 1), path)
            })
            .collect();
        Ok(BookArchive::spawn(
            format!("{}.zip", read_list.name),
            entries,
        ))
    }
}
