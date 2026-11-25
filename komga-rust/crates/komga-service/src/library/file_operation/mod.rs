//! Files written next to or over library files. The filesystem and SQLite share no transaction,
//! so an operation is recorded with the version of its source before anything is written; its
//! output goes to a temporary file in the target's directory and, once verified, is published by
//! a rename. The database changes using the output are committed with a mark on the record; the
//! source is removed only after that. At start, operations an earlier run left are completed when
//! marked and undone otherwise, as their records and the files on disk show.

mod removal;
mod rename;
#[cfg(test)]
mod tests;

use std::fs::{self, Metadata, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use komga_db::{Database, DbError, DbTransaction, FileOperation, FileOperationDao, FileVersion};
use komga_types::FileUrl;
use thiserror::Error;
use tokio::task::{self, JoinError};
use tracing::warn;

#[derive(Debug, Error)]
pub enum FileOperationError {
    #[error("Destination file already exists: {}", .0.display())]
    TargetExists(PathBuf),
    #[error("Source file changed since the operation began: {}", .0.display())]
    SourceChanged(PathBuf),
    #[error("Output file changed or removed before it was committed: {}", .0.display())]
    OutputLost(PathBuf),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("Filesystem worker failed: {0}")]
    Worker(#[from] JoinError),
}

#[derive(Clone)]
pub struct FileOperations {
    db: Database,
}

/// What becomes of the source once the output is committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAfter {
    Keep,
    /// Removed if still at the version the operation began with; a move, across filesystems too.
    Remove,
}

/// A recorded operation writing `target` from `source`.
#[derive(Debug, Clone)]
pub struct PendingFile {
    record: FileOperation,
    source: PathBuf,
    target: PathBuf,
    temporary: PathBuf,
    /// The replaced file and where it is set aside.
    replaced: Option<(PathBuf, PathBuf)>,
}

/// What an operation left on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileState {
    pub output: OutputState,
    pub source: SourceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputState {
    /// Not verified: a temporary file left is incomplete.
    Unverified,
    /// Verified, in its temporary file.
    Verified,
    /// Verified, at the target.
    Published,
    /// Verified, then changed or removed by someone else.
    Lost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceState {
    /// At the version the operation began with.
    Unchanged,
    /// At another version, which is the output once it replaced the source.
    Changed,
    Missing,
}

impl FileOperations {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Records an operation writing the absolute `target` from the absolute `source` at its
    /// current version. The output supersedes the absolute `replaced`, if any: that file is set
    /// aside when the output is published, removed once it is committed, and put back otherwise.
    /// `target` must not exist, unless it is `replaced`, or `source`, which the output then
    /// replaces and which `source_after` cannot remove.
    pub async fn begin(
        &self,
        source: &Path,
        target: &Path,
        replaced: Option<&Path>,
        source_after: SourceAfter,
    ) -> Result<PendingFile, FileOperationError> {
        let (source, target) = (source.to_path_buf(), target.to_path_buf());
        let replaces_target = replaced == Some(target.as_path());
        let (source, target, source_version, target_exists) = task::spawn_blocking(move || {
            let version = version(&fs::metadata(&source)?)?;
            let target_exists = source != target && !replaces_target && fs::exists(&target)?;
            io::Result::Ok((source, target, version, target_exists))
        })
        .await??;
        if target_exists {
            return Err(FileOperationError::TargetExists(target));
        }
        let mut record = FileOperation {
            id: 0,
            source: FileUrl::file(&source),
            source_version,
            target: FileUrl::file(&target),
            replaced: replaced.map(FileUrl::file),
            remove_source: source_after == SourceAfter::Remove,
            output_version: None,
            committed: false,
        };
        let mut tx = self.db.begin().await?;
        record.id = FileOperationDao::insert(&mut tx, &record).await?;
        tx.commit().await?;
        Ok(PendingFile::new(record))
    }

    /// Publishes the output written to [`PendingFile::temporary`], which the caller has verified.
    /// Fails, leaving the output in place, when the source changed or the target appeared.
    pub async fn publish(&self, file: &mut PendingFile) -> Result<(), FileOperationError> {
        let temporary = file.temporary.clone();
        let output = task::spawn_blocking(move || {
            let output = OpenOptions::new().write(true).open(&temporary)?;
            output.sync_all()?;
            version(&output.metadata()?)
        })
        .await??;
        if file.state().await?.source != SourceState::Unchanged {
            return Err(FileOperationError::SourceChanged(file.source.clone()));
        }
        // Recorded first, so that after a crash the published output is told from another file.
        let mut tx = self.db.begin().await?;
        FileOperationDao::set_output(&mut tx, file.record.id, output).await?;
        tx.commit().await?;
        file.record.output_version = Some(output);
        let pending = file.clone();
        let renamed = task::spawn_blocking(move || {
            if let Some((replaced, aside)) = &pending.replaced
                && fs::exists(replaced)?
            {
                rename::rename(replaced, aside, true)?;
            }
            let replace = pending.source == pending.target;
            rename::rename(&pending.temporary, &pending.target, replace)
        })
        .await?;
        match renamed {
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                Err(FileOperationError::TargetExists(file.target.clone()))
            }
            renamed => Ok(renamed?),
        }
    }

    /// Marks `file` committed with `transaction`, which carries the database changes that use its
    /// published output: from then on the operation is completed rather than undone. Fails when
    /// the output is no longer at the target or, unless the output replaced it, the source changed.
    pub async fn commit(
        &self,
        transaction: &mut DbTransaction,
        file: &PendingFile,
    ) -> Result<(), FileOperationError> {
        let state = file.state().await?;
        if state.output != OutputState::Published {
            return Err(FileOperationError::OutputLost(file.target.clone()));
        }
        if file.source != file.target && state.source != SourceState::Unchanged {
            return Err(FileOperationError::SourceChanged(file.source.clone()));
        }
        FileOperationDao::set_committed(transaction, file.record.id).await?;
        Ok(())
    }

    /// The operations recorded and not finished, oldest first.
    pub async fn recorded(&self) -> Result<Vec<PendingFile>, FileOperationError> {
        let mut tx = self.db.begin().await?;
        let records = FileOperationDao::find_all(&mut tx).await?;
        tx.commit().await?;
        Ok(records.into_iter().map(PendingFile::new).collect())
    }

    /// Completes `file` when it is committed, removing the source if asked and it is unchanged and
    /// the replaced file, and undoes it otherwise, removing its output unless the output replaced
    /// the source and putting the replaced file back; then removes the record. Files others
    /// changed since stay.
    pub async fn finish(&self, file: PendingFile) -> Result<(), FileOperationError> {
        // Read again: a commit marks the database, not `file`, and its transaction may have failed.
        let mut tx = self.db.begin().await?;
        let record = FileOperationDao::find(&mut tx, file.record.id).await?;
        tx.commit().await?;
        if let Some(record) = record {
            let id = record.id;
            let file = PendingFile::new(record);
            task::spawn_blocking(move || file.clean_up()).await??;
            let mut tx = self.db.begin().await?;
            FileOperationDao::delete(&mut tx, id).await?;
            tx.commit().await?;
        }
        Ok(())
    }

    /// Finishes the operations an earlier run left. Run it at start, before the tasks that may
    /// begin operations on the same files again; an operation that fails stays recorded.
    pub async fn recover(&self) {
        let recorded = match self.recorded().await {
            Ok(recorded) => recorded,
            Err(error) => return warn!(%error, "Failed to read the file operations to recover"),
        };
        for file in recorded {
            let target = file.target.clone();
            if let Err(error) = self.finish(file).await {
                warn!(%error, target = %target.display(), "Failed to recover a file operation");
            }
        }
    }
}

impl PendingFile {
    fn new(record: FileOperation) -> Self {
        let source = record.source.to_path().expect("Recorded from a local path");
        let target = record.target.to_path().expect("Recorded from a local path");
        // Hidden, without a book extension, and named by the operation: scans skip it, and a long
        // target name cannot make it too long.
        let temporary = target.with_file_name(format!(".komga-{}.tmp", record.id));
        let replaced = record.replaced.as_ref().map(|replaced| {
            let replaced = replaced.to_path().expect("Recorded from a local path");
            let aside = replaced.with_file_name(format!(".komga-{}.old", record.id));
            (replaced, aside)
        });
        Self {
            record,
            source,
            target,
            temporary,
            replaced,
        }
    }

    #[must_use]
    pub fn source(&self) -> &Path {
        &self.source
    }

    #[must_use]
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// Where to write the output: the target's directory, so that publishing it is a rename.
    #[must_use]
    pub fn temporary(&self) -> &Path {
        &self.temporary
    }

    /// Copies the source to [`Self::temporary`], for an output that is the source elsewhere, such
    /// as on another filesystem. Fails when the source is not at its recorded version throughout.
    pub async fn copy_source(&self) -> Result<(), FileOperationError> {
        let file = self.clone();
        let unchanged = task::spawn_blocking(move || {
            let version = Some(file.record.source_version);
            if stat(&file.source)? != version {
                return io::Result::Ok(false);
            }
            let copied = fs::copy(&file.source, &file.temporary)?;
            Ok(copied.cast_signed() == file.record.source_version.size
                && stat(&file.source)? == version)
        })
        .await??;
        if unchanged {
            Ok(())
        } else {
            Err(FileOperationError::SourceChanged(self.source.clone()))
        }
    }

    /// What the operation left, from its record and the files on disk.
    pub async fn state(&self) -> Result<FileState, FileOperationError> {
        let file = self.clone();
        Ok(task::spawn_blocking(move || file.read_state()).await??)
    }

    fn read_state(&self) -> io::Result<FileState> {
        let output = match self.record.output_version {
            None => OutputState::Unverified,
            Some(output) if stat(&self.temporary)? == Some(output) => OutputState::Verified,
            Some(output) if stat(&self.target)? == Some(output) => OutputState::Published,
            Some(_) => OutputState::Lost,
        };
        let source = match stat(&self.source)? {
            None => SourceState::Missing,
            Some(version) if version == self.record.source_version => SourceState::Unchanged,
            Some(_) => SourceState::Changed,
        };
        Ok(FileState { output, source })
    }

    fn clean_up(&self) -> io::Result<()> {
        let state = self.read_state()?;
        remove(&self.temporary)?;
        if self.record.committed {
            // An output that replaced its source leaves no unchanged source to remove.
            if self.record.remove_source {
                let published = state.output == OutputState::Published;
                match state.source {
                    SourceState::Unchanged if published => remove(&self.source)?,
                    SourceState::Missing => {}
                    _ => warn!(
                        source = %self.source.display(),
                        "Keeping the source of a moved file: it or the output changed since"
                    ),
                }
            }
            if let Some((_, aside)) = &self.replaced {
                remove(aside)?;
            }
        } else {
            let aside = match &self.replaced {
                Some((replaced, aside)) if fs::exists(aside)? => Some((replaced, aside)),
                _ => None,
            };
            // An output that replaced its source stays, unless the original was set aside.
            if state.output == OutputState::Published
                && (self.source != self.target || aside.is_some())
            {
                remove(&self.target)?;
            }
            if let Some((replaced, aside)) = aside {
                rename::rename(aside, replaced, false)?;
            }
        }
        Ok(())
    }
}

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        removed => removed,
    }
}

fn stat(path: &Path) -> io::Result<Option<FileVersion>> {
    match fs::metadata(path) {
        Ok(metadata) => version(&metadata).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn version(metadata: &Metadata) -> io::Result<FileVersion> {
    let modified = metadata.modified()?;
    let nanos = match modified.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_nanos()),
        Err(before) => i64::try_from(before.duration().as_nanos()).map(|nanos| -nanos),
    };
    let modified = nanos.map_err(|_| io::Error::other("File time out of range"))?;
    Ok(FileVersion {
        size: metadata.len().cast_signed(),
        modified,
    })
}
