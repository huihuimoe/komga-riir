//! Kotlin `BookController.getBookPositions`.

use komga_db::MediaDao;
use komga_types::{KomgaUser, R2Locator};

use super::{BookSource, ReadingError, ReadingService};

impl ReadingService {
    /// The positions found when the book was analyzed as an EPUB; not found without a stored
    /// EPUB extension.
    pub async fn positions(
        &self,
        user: &KomgaUser,
        source: BookSource,
    ) -> Result<Vec<R2Locator>, ReadingError> {
        self.check_access(user, &source.book).await?;
        let mut tx = self.db.begin().await?;
        let extension = MediaDao::find_extension(&mut tx, &source.book.id).await?;
        tx.rollback().await?;
        extension
            .map(|extension| extension.positions)
            .ok_or(ReadingError::NotFound)
    }
}
