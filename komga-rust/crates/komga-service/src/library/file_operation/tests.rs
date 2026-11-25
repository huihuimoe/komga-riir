use std::fs;
use std::path::PathBuf;

use komga_db::Database;
use komga_test_support::TestDir;

use super::{
    FileOperationError, FileOperations, FileState, OutputState, PendingFile, SourceAfter,
    SourceState,
};

struct Fixture {
    directory: TestDir,
    source: PathBuf,
    target: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = komga_test_support::tempdir().expect("Directory");
        let source = directory.path().join("book.cbr");
        let target = directory.path().join("book.cbz");
        fs::write(&source, b"source").expect("Source");
        Self {
            directory,
            source,
            target,
        }
    }

    /// The service as at a start: everything it knows is in the database.
    async fn operations(&self) -> FileOperations {
        let db = Database::connect(&self.directory.path().join("database.sqlite"))
            .await
            .expect("Open database");
        db.migrate().await.expect("Migrate database");
        FileOperations::new(db)
    }

    /// The single operation recorded, as read back after a restart.
    async fn restarted(&self) -> (FileOperations, PendingFile, FileState) {
        let operations = self.operations().await;
        let mut recorded = operations.recorded().await.expect("Recorded operations");
        assert_eq!(recorded.len(), 1, "one operation is recorded");
        let file = recorded.remove(0);
        let state = file.state().await.expect("State");
        (operations, file, state)
    }
}

fn state(output: OutputState, source: SourceState) -> FileState {
    FileState { output, source }
}

async fn commit(operations: &FileOperations, file: &PendingFile) -> Result<(), FileOperationError> {
    let mut tx = operations.db.begin_immediate().await?;
    operations.commit(&mut tx, file).await?;
    tx.commit().await?;
    Ok(())
}

#[tokio::test]
async fn an_interrupted_write_is_abandoned_without_touching_the_source() {
    let fixture = Fixture::new();
    let operations = fixture.operations().await;
    let file = operations
        .begin(&fixture.source, &fixture.target, None, SourceAfter::Keep)
        .await
        .expect("Begin");
    fs::write(file.temporary(), b"half").expect("Partial output");

    let (operations, file, found) = fixture.restarted().await;
    assert_eq!(
        found,
        state(OutputState::Unverified, SourceState::Unchanged)
    );
    let temporary = file.temporary().to_path_buf();
    operations.finish(file).await.expect("Finish");
    assert!(!temporary.exists(), "the partial output is removed");
    assert_eq!(fs::read(&fixture.source).expect("Source"), b"source");
    assert!(!fixture.target.exists());
    let recorded = operations.recorded().await.expect("Recorded operations");
    assert!(recorded.is_empty(), "the record is removed");
}

#[tokio::test]
async fn a_published_output_is_recognized_after_a_restart() {
    let fixture = Fixture::new();
    let operations = fixture.operations().await;
    let mut file = operations
        .begin(&fixture.source, &fixture.target, None, SourceAfter::Keep)
        .await
        .expect("Begin");
    fs::write(file.temporary(), b"output").expect("Output");
    operations.publish(&mut file).await.expect("Publish");
    assert!(!file.temporary().exists(), "the output is moved");

    let (operations, file, found) = fixture.restarted().await;
    assert_eq!(found, state(OutputState::Published, SourceState::Unchanged));
    commit(&operations, &file).await.expect("Commit");
    operations.finish(file).await.expect("Finish");
    assert_eq!(fs::read(&fixture.target).expect("Target"), b"output");
    assert_eq!(fs::read(&fixture.source).expect("Source"), b"source");

    // Replaced since, the output is no longer the one published.
    let mut file = operations
        .begin(
            &fixture.source,
            &fixture.directory.path().join("other.cbz"),
            None,
            SourceAfter::Keep,
        )
        .await
        .expect("Begin another");
    fs::write(file.temporary(), b"output").expect("Output");
    operations.publish(&mut file).await.expect("Publish");
    fs::write(file.target(), b"replaced by someone else").expect("Replace");
    let (_, _, found) = fixture.restarted().await;
    assert_eq!(found, state(OutputState::Lost, SourceState::Unchanged));
}

#[tokio::test]
async fn a_target_that_appears_keeps_the_verified_output_unpublished() {
    let fixture = Fixture::new();
    let operations = fixture.operations().await;
    let mut file = operations
        .begin(&fixture.source, &fixture.target, None, SourceAfter::Keep)
        .await
        .expect("Begin");
    fs::write(file.temporary(), b"output").expect("Output");
    fs::write(&fixture.target, b"another file").expect("Another file at the target");

    let published = operations.publish(&mut file).await;
    assert!(matches!(
        published,
        Err(FileOperationError::TargetExists(_))
    ));
    assert_eq!(fs::read(&fixture.target).expect("Target"), b"another file");
    let (operations, mut file, found) = fixture.restarted().await;
    assert_eq!(found, state(OutputState::Verified, SourceState::Unchanged));

    fs::remove_file(&fixture.target).expect("Remove the other file");
    operations.publish(&mut file).await.expect("Publish");
    assert_eq!(fs::read(&fixture.target).expect("Target"), b"output");
}

#[tokio::test]
async fn an_existing_target_or_a_changed_source_is_refused() {
    let fixture = Fixture::new();
    let operations = fixture.operations().await;
    fs::write(&fixture.target, b"existing").expect("Existing target");
    let begun = operations
        .begin(&fixture.source, &fixture.target, None, SourceAfter::Keep)
        .await;
    assert!(matches!(begun, Err(FileOperationError::TargetExists(_))));
    assert!(operations.recorded().await.expect("Recorded").is_empty());
    fs::remove_file(&fixture.target).expect("Remove the target");

    let mut file = operations
        .begin(&fixture.source, &fixture.target, None, SourceAfter::Keep)
        .await
        .expect("Begin");
    fs::write(file.temporary(), b"output").expect("Output");
    fs::write(&fixture.source, b"changed source").expect("Change the source");
    let published = operations.publish(&mut file).await;
    assert!(matches!(
        published,
        Err(FileOperationError::SourceChanged(_))
    ));
    assert!(!fixture.target.exists(), "nothing is published");
    let (_, _, found) = fixture.restarted().await;
    assert_eq!(found, state(OutputState::Unverified, SourceState::Changed));
}

#[tokio::test]
async fn an_output_replaces_its_source_only_when_it_is_the_target() {
    let fixture = Fixture::new();
    let operations = fixture.operations().await;
    let mut file = operations
        .begin(&fixture.source, &fixture.source, None, SourceAfter::Keep)
        .await
        .expect("Begin");
    fs::write(file.temporary(), b"edited").expect("Output");
    operations.publish(&mut file).await.expect("Publish");

    assert_eq!(fs::read(&fixture.source).expect("Source"), b"edited");
    let (_, _, found) = fixture.restarted().await;
    assert_eq!(found, state(OutputState::Published, SourceState::Changed));
}

impl Fixture {
    /// A move of the source to the target, written and published.
    async fn published_move(&self) -> (FileOperations, PendingFile) {
        let operations = self.operations().await;
        let mut file = operations
            .begin(&self.source, &self.target, None, SourceAfter::Remove)
            .await
            .expect("Begin");
        fs::write(file.temporary(), b"output").expect("Output");
        operations.publish(&mut file).await.expect("Publish");
        (operations, file)
    }

    /// Recovers as at a start, after which nothing stays recorded.
    async fn recover(&self) {
        let operations = self.operations().await;
        operations.recover().await;
        let recorded = operations.recorded().await.expect("Recorded operations");
        assert!(recorded.is_empty(), "every operation is finished");
    }

    fn contents(&self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        (fs::read(&self.source).ok(), fs::read(&self.target).ok())
    }
}

fn contents(source: Option<&[u8]>, target: Option<&[u8]>) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    (source.map(<[u8]>::to_vec), target.map(<[u8]>::to_vec))
}

#[tokio::test]
async fn an_operation_left_before_its_commit_is_undone_at_recovery() {
    // Written, not verified.
    let fixture = Fixture::new();
    let operations = fixture.operations().await;
    let file = operations
        .begin(&fixture.source, &fixture.target, None, SourceAfter::Remove)
        .await
        .expect("Begin");
    fs::write(file.temporary(), b"half").expect("Partial output");
    fixture.recover().await;
    assert!(!file.temporary().exists());
    assert_eq!(fixture.contents(), contents(Some(b"source"), None));

    // Verified, not published.
    let mut file = operations
        .begin(&fixture.source, &fixture.target, None, SourceAfter::Remove)
        .await
        .expect("Begin");
    fs::write(file.temporary(), b"output").expect("Output");
    fs::write(&fixture.target, b"blocker").expect("Block the target");
    assert!(operations.publish(&mut file).await.is_err());
    fs::remove_file(&fixture.target).expect("Unblock the target");
    fixture.recover().await;
    assert!(!file.temporary().exists());
    assert_eq!(fixture.contents(), contents(Some(b"source"), None));

    // Published, not committed.
    fixture.published_move().await;
    fixture.recover().await;
    assert_eq!(fixture.contents(), contents(Some(b"source"), None));
}

#[tokio::test]
async fn a_committed_move_removes_its_source_at_recovery_unless_either_changed() {
    let fixture = Fixture::new();
    let (operations, file) = fixture.published_move().await;
    commit(&operations, &file).await.expect("Commit");
    fixture.recover().await;
    assert_eq!(fixture.contents(), contents(None, Some(b"output")));

    fs::rename(&fixture.target, &fixture.source).expect("Reset");
    let (operations, file) = fixture.published_move().await;
    commit(&operations, &file).await.expect("Commit");
    fs::write(&fixture.source, b"changed source").expect("Change the source");
    fixture.recover().await;
    assert_eq!(
        fixture.contents(),
        contents(Some(b"changed source"), Some(b"output"))
    );

    fs::remove_file(&fixture.target).expect("Reset");
    let (operations, file) = fixture.published_move().await;
    commit(&operations, &file).await.expect("Commit");
    fs::remove_file(&fixture.target).expect("Remove the output");
    fixture.recover().await;
    assert_eq!(fixture.contents(), contents(Some(b"changed source"), None));
}

#[tokio::test]
async fn a_commit_is_refused_after_the_source_or_the_output_changed() {
    let fixture = Fixture::new();
    let (operations, file) = fixture.published_move().await;
    fs::write(&fixture.source, b"changed source").expect("Change the source");
    let committed = commit(&operations, &file).await;
    assert!(matches!(
        committed,
        Err(FileOperationError::SourceChanged(_))
    ));
    fixture.recover().await;
    assert_eq!(fixture.contents(), contents(Some(b"changed source"), None));

    let (operations, file) = fixture.published_move().await;
    fs::write(&fixture.target, b"replaced output").expect("Replace the output");
    let committed = commit(&operations, &file).await;
    assert!(matches!(committed, Err(FileOperationError::OutputLost(_))));
    fixture.recover().await;
    assert_eq!(
        fixture.contents(),
        contents(Some(b"changed source"), Some(b"replaced output"))
    );
}

#[tokio::test]
async fn a_commit_whose_transaction_fails_is_undone() {
    let fixture = Fixture::new();
    let (operations, file) = fixture.published_move().await;
    let mut tx = operations.db.begin_immediate().await.expect("Transaction");
    operations.commit(&mut tx, &file).await.expect("Commit");
    tx.rollback().await.expect("Roll back");
    operations.finish(file).await.expect("Finish");
    assert_eq!(fixture.contents(), contents(Some(b"source"), None));
}

#[tokio::test]
async fn an_output_that_replaced_its_source_stays() {
    let fixture = Fixture::new();
    for committed in [false, true] {
        let operations = fixture.operations().await;
        let mut file = operations
            .begin(&fixture.source, &fixture.source, None, SourceAfter::Remove)
            .await
            .expect("Begin");
        fs::write(file.temporary(), b"edited").expect("Output");
        operations.publish(&mut file).await.expect("Publish");
        if committed {
            commit(&operations, &file).await.expect("Commit");
        }
        fixture.recover().await;
        assert_eq!(fs::read(&fixture.source).expect("Source"), b"edited");
        fs::write(&fixture.source, b"source").expect("Reset");
    }
}

#[tokio::test]
async fn a_copied_source_is_moved_once_verified_unchanged() {
    let fixture = Fixture::new();
    let operations = fixture.operations().await;
    let mut file = operations
        .begin(&fixture.source, &fixture.target, None, SourceAfter::Remove)
        .await
        .expect("Begin");
    file.copy_source().await.expect("Copy");
    operations.publish(&mut file).await.expect("Publish");
    commit(&operations, &file).await.expect("Commit");
    operations.finish(file).await.expect("Finish");
    assert_eq!(fixture.contents(), contents(None, Some(b"source")));

    fs::rename(&fixture.target, &fixture.source).expect("Reset");
    let file = operations
        .begin(&fixture.source, &fixture.target, None, SourceAfter::Remove)
        .await
        .expect("Begin");
    fs::write(&fixture.source, b"changed source").expect("Change the source");
    let copied = file.copy_source().await;
    assert!(matches!(copied, Err(FileOperationError::SourceChanged(_))));
}

#[tokio::test]
async fn a_replaced_file_is_put_back_on_undo_and_removed_once_committed() {
    let fixture = Fixture::new();
    let operations = fixture.operations().await;
    let replace_target = async || {
        fs::write(&fixture.target, b"old").expect("Old file");
        let mut file = operations
            .begin(
                &fixture.source,
                &fixture.target,
                Some(&fixture.target),
                SourceAfter::Keep,
            )
            .await
            .expect("Begin");
        fs::write(file.temporary(), b"output").expect("Output");
        operations.publish(&mut file).await.expect("Publish");
        file
    };

    replace_target().await;
    fixture.recover().await;
    assert_eq!(fixture.contents(), contents(Some(b"source"), Some(b"old")));

    let file = replace_target().await;
    commit(&operations, &file).await.expect("Commit");
    operations.finish(file).await.expect("Finish");
    assert_eq!(
        fixture.contents(),
        contents(Some(b"source"), Some(b"output"))
    );
    let left: Vec<_> = fs::read_dir(fixture.directory.path())
        .expect("Directory")
        .map(|entry| entry.expect("Entry").file_name())
        .filter(|name| name.to_string_lossy().starts_with(".komga-"))
        .collect();
    assert!(left.is_empty(), "nothing is left aside: {left:?}");
}

#[tokio::test]
async fn an_output_replacing_its_set_aside_source_is_undone_until_committed() {
    let fixture = Fixture::new();
    for committed in [false, true] {
        let operations = fixture.operations().await;
        let mut file = operations
            .begin(
                &fixture.source,
                &fixture.source,
                Some(&fixture.source),
                SourceAfter::Keep,
            )
            .await
            .expect("Begin");
        fs::write(file.temporary(), b"edited").expect("Output");
        operations.publish(&mut file).await.expect("Publish");
        if committed {
            commit(&operations, &file).await.expect("Commit");
        }
        fixture.recover().await;
        let expected: &[u8] = if committed { b"edited" } else { b"source" };
        assert_eq!(fixture.contents(), contents(Some(expected), None));
        fs::write(&fixture.source, b"source").expect("Reset");
    }
    let left: Vec<_> = fs::read_dir(fixture.directory.path())
        .expect("Directory")
        .map(|entry| entry.expect("Entry").file_name())
        .filter(|name| name.to_string_lossy().starts_with(".komga-"))
        .collect();
    assert!(left.is_empty(), "nothing is left aside: {left:?}");
}

#[tokio::test]
async fn a_removed_source_comes_back_until_committed_and_a_changed_one_is_kept() {
    let fixture = Fixture::new();
    for committed in [false, true] {
        let operations = fixture.operations().await;
        let source = Some(fixture.source.as_path());
        let file = operations
            .begin(&fixture.source, &fixture.source, source, SourceAfter::Keep)
            .await
            .expect("Begin");
        operations.set_aside(&file).await.expect("Set aside");
        assert!(!fixture.source.exists());
        if committed {
            let mut tx = operations.db.begin_immediate().await.expect("Transaction");
            operations
                .commit_removal(&mut tx, &file)
                .await
                .expect("Commit");
            tx.commit().await.expect("Commit transaction");
        }
        fixture.recover().await;
        let expected = (!committed).then_some(b"source".as_slice());
        assert_eq!(fixture.contents(), contents(expected, None));
    }

    fs::write(&fixture.source, b"source").expect("Source");
    let operations = fixture.operations().await;
    let source = Some(fixture.source.as_path());
    let file = operations
        .begin(&fixture.source, &fixture.source, source, SourceAfter::Keep)
        .await
        .expect("Begin");
    fs::write(&fixture.source, b"changed").expect("Change");
    let refused = operations.set_aside(&file).await;
    assert!(matches!(refused, Err(FileOperationError::SourceChanged(_))));
    operations.finish(file).await.expect("Finish");
    assert_eq!(fixture.contents(), contents(Some(b"changed"), None));
}
