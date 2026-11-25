//! Removing a file through an operation: set aside, then gone once committed.

use std::io;

use komga_db::{DbTransaction, FileOperationDao};
use tokio::task;

use super::{FileOperationError, FileOperations, PendingFile, rename, stat};

impl FileOperations {
    /// Sets the replaced source of `file`, which has no output, aside: committing it removes the
    /// source, and undoing it puts it back. Fails when the source changed.
    pub async fn set_aside(&self, file: &PendingFile) -> Result<(), FileOperationError> {
        let pending = file.clone();
        let set_aside = task::spawn_blocking(move || {
            let (replaced, aside) = pending.replaced.as_ref().expect("A replaced source");
            if stat(replaced)? != Some(pending.record.source_version) {
                return io::Result::Ok(false);
            }
            rename::rename(replaced, aside, false)?;
            Ok(true)
        })
        .await??;
        if set_aside {
            Ok(())
        } else {
            Err(FileOperationError::SourceChanged(file.source.clone()))
        }
    }

    /// Marks the removal of the source [`Self::set_aside`] moved committed with `transaction`.
    pub async fn commit_removal(
        &self,
        transaction: &mut DbTransaction,
        file: &PendingFile,
    ) -> Result<(), FileOperationError> {
        Ok(FileOperationDao::set_committed(transaction, file.record.id).await?)
    }
}
