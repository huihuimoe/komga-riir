use super::{ClaimedTask, TaskEnd, TaskRow, TasksDao};
use crate::Database;

const CLASS: &str = "org.gotson.komga.application.tasks.Task$AnalyzeBook";

fn row(id: &str, priority: i32, group: Option<&str>) -> TaskRow {
    TaskRow {
        id: id.to_owned(),
        priority,
        group_id: group.map(str::to_owned),
        class: CLASS.to_owned(),
        simple_type: "AnalyzeBook".to_owned(),
        payload: format!(r#"{{"bookId":"{id}"}}"#),
    }
}

async fn queue(rows: &[TaskRow]) -> (komga_test_support::TestDir, Database) {
    let directory = komga_test_support::tempdir().expect("Tasks directory");
    let db = Database::connect(&directory.path().join("tasks.sqlite"))
        .await
        .expect("Open tasks database");
    db.migrate_tasks().await.expect("Migrate tasks database");
    for row in rows {
        save(&db, row).await;
    }
    (directory, db)
}

async fn save(db: &Database, row: &TaskRow) {
    let mut tx = db.begin().await.expect("Begin");
    TasksDao::save(&mut tx, row).await.expect("Save task");
    tx.commit().await.expect("Commit");
}

async fn claim(db: &Database, classes: &[&str]) -> Option<ClaimedTask> {
    let mut tx = db.begin_immediate().await.expect("Begin");
    let task = TasksDao::take_first(&mut tx, "worker", classes)
        .await
        .expect("Take task");
    tx.commit().await.expect("Commit");
    task
}

async fn take(db: &Database, classes: &[&str]) -> Option<String> {
    claim(db, classes).await.map(|task| task.id)
}

async fn finish(db: &Database, task: &ClaimedTask, end: TaskEnd<'_>) {
    let mut tx = db.begin().await.expect("Begin");
    TasksDao::finish(&mut tx, &task.id, task.version, end)
        .await
        .expect("Finish task");
    tx.commit().await.expect("Commit");
}

async fn count(db: &Database) -> i64 {
    let mut tx = db.begin().await.expect("Begin");
    TasksDao::count(&mut tx).await.expect("Count")
}

#[tokio::test]
async fn claims_by_priority_then_submission_order_and_skips_busy_groups() {
    let (_directory, db) = queue(&[
        row("low", 2, None),
        row("first", 6, Some("series-a")),
        row("same-group", 6, Some("series-a")),
        row("second", 6, Some("series-b")),
    ])
    .await;
    let first = claim(&db, &[CLASS]).await.expect("First task");
    let mut claimed = vec![first.id.clone()];
    while let Some(id) = take(&db, &[CLASS]).await {
        claimed.push(id);
    }
    assert_eq!(
        claimed,
        ["first", "second", "low"],
        "series-a stays busy while its first task is owned"
    );
    finish(&db, &first, TaskEnd::Succeeded).await;
    assert_eq!(take(&db, &[CLASS]).await.as_deref(), Some("same-group"));
}

#[tokio::test]
async fn resubmission_replaces_content_and_requeues_without_duplicating() {
    let (_directory, db) = queue(&[row("a", 4, None), row("b", 4, None)]).await;
    let mut replaced = row("a", 6, None);
    replaced.payload = r#"{"bookId":"a","replaced":true}"#.to_owned();
    save(&db, &replaced).await;
    let task = claim(&db, &[CLASS]).await.expect("A task is available");
    assert_eq!((task.id.as_str(), task.version), ("a", 1));
    assert!(
        task.payload.contains(r#""replaced":true"#),
        "{}",
        task.payload
    );
    assert_eq!(take(&db, &[CLASS]).await.as_deref(), Some("b"));
    assert_eq!(take(&db, &[CLASS]).await, None);
    assert_eq!(count(&db).await, 2);
}

#[tokio::test]
async fn a_task_resubmitted_while_running_runs_again() {
    let (_directory, db) = queue(&[row("a", 4, None)]).await;
    let running = claim(&db, &[CLASS]).await.expect("Running task");
    save(&db, &row("a", 4, None)).await;
    assert_eq!(take(&db, &[CLASS]).await, None, "the row is still owned");
    finish(&db, &running, TaskEnd::Succeeded).await;
    let again = claim(&db, &[CLASS]).await.expect("Resubmitted task");
    assert_eq!((again.version, again.attempts), (1, 1));
    finish(&db, &again, TaskEnd::Succeeded).await;
    assert_eq!(count(&db).await, 0);
}

#[tokio::test]
async fn retries_wait_until_due_and_failures_stay_until_resubmitted() {
    let (_directory, db) = queue(&[row("a", 4, None)]).await;
    let task = claim(&db, &[CLASS]).await.expect("Task");
    let retry = TaskEnd::Retry {
        delay_seconds: 3600,
        failure: "busy",
    };
    finish(&db, &task, retry).await;
    assert_eq!(take(&db, &[CLASS]).await, None, "not due yet");
    let mut tx = db.begin().await.expect("Begin");
    let due = TasksDao::next_retry_in(&mut tx).await.expect("Next retry");
    drop(tx);
    assert!(
        due.is_some_and(|seconds| (3590.0..=3600.0).contains(&seconds)),
        "{due:?}"
    );

    let retry = TaskEnd::Retry {
        delay_seconds: 0,
        failure: "busy",
    };
    finish(&db, &task, retry).await;
    let task = claim(&db, &[CLASS]).await.expect("Due retry");
    assert_eq!(task.attempts, 2);
    finish(&db, &task, TaskEnd::Failed { failure: "corrupt" }).await;
    assert_eq!(
        take(&db, &[CLASS]).await,
        None,
        "failed tasks are not claimed"
    );
    assert_eq!(count(&db).await, 1, "the failure stays recorded");
    let mut tx = db.begin().await.expect("Begin");
    assert_eq!(
        TasksDao::next_retry_in(&mut tx).await.expect("Next retry"),
        None
    );
    drop(tx);

    save(&db, &row("a", 4, None)).await;
    let task = claim(&db, &[CLASS]).await.expect("Resubmitted failure");
    assert_eq!(task.attempts, 1);
    assert!(!task.payload.contains("eraFailure"), "{}", task.payload);
}

#[tokio::test]
async fn tasks_stopped_for_a_shutdown_are_queued_without_using_an_attempt() {
    let (_directory, db) = queue(&[row("a", 4, None)]).await;
    for _ in 0..2 {
        let task = claim(&db, &[CLASS]).await.expect("Queued task");
        assert_eq!(task.attempts, 1);
        finish(&db, &task, TaskEnd::Interrupted).await;
    }
    assert_eq!(count(&db).await, 1);
}

#[tokio::test]
async fn disown_releases_interrupted_tasks_until_attempts_run_out() {
    let (_directory, db) = queue(&[row("a", 4, None)]).await;
    for (attempt, expected) in [(1, (1, 0)), (2, (0, 1))] {
        let task = claim(&db, &[CLASS]).await.expect("Interrupted task");
        assert_eq!(task.attempts, attempt);
        let mut tx = db.begin().await.expect("Begin");
        assert_eq!(
            TasksDao::disown(&mut tx, 2).await.expect("Disown"),
            expected
        );
        tx.commit().await.expect("Commit");
    }
    assert_eq!(take(&db, &[CLASS]).await, None, "exhausted tasks fail");
    assert_eq!(count(&db).await, 1);
}

#[tokio::test]
async fn rows_of_unknown_classes_stay_queued() {
    let mut unknown = row("legacy", 8, None);
    unknown.class = "org.gotson.komga.application.tasks.Task$ScanLibrary".to_owned();
    let (_directory, db) = queue(&[unknown, row("known", 0, None)]).await;
    assert_eq!(take(&db, &[CLASS]).await.as_deref(), Some("known"));
    assert_eq!(take(&db, &[CLASS]).await, None);
    let both = [CLASS, "org.gotson.komga.application.tasks.Task$ScanLibrary"];
    assert_eq!(take(&db, &both).await.as_deref(), Some("legacy"));
}

#[tokio::test]
async fn clearing_the_queue_keeps_running_and_failed_tasks() {
    let (_directory, db) = queue(&[
        row("running", 6, None),
        row("failed", 6, None),
        row("retrying", 6, None),
        row("queued", 6, None),
    ])
    .await;
    let running = claim(&db, &[CLASS]).await.expect("Running task");
    let failed = claim(&db, &[CLASS]).await.expect("Failed task");
    finish(&db, &failed, TaskEnd::Failed { failure: "broken" }).await;
    let retrying = claim(&db, &[CLASS]).await.expect("Retrying task");
    let retry = TaskEnd::Retry {
        delay_seconds: 60,
        failure: "busy",
    };
    finish(&db, &retrying, retry).await;

    let mut tx = db.begin().await.expect("Begin");
    let deleted = TasksDao::delete_queued(&mut tx).await.expect("Clear queue");
    tx.commit().await.expect("Commit");
    assert_eq!(deleted, 2, "the queued and the retrying task");
    assert_eq!(count(&db).await, 2);
    finish(&db, &running, TaskEnd::Succeeded).await;
    assert_eq!(take(&db, &[CLASS]).await, None, "nothing comes back");
    assert_eq!(count(&db).await, 1, "only the failed task stays");
}
