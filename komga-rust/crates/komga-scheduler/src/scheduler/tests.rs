use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use komga_db::{Database, TasksDao};
use tokio::sync::{Semaphore, mpsc};
use tokio::time;
use tokio_util::sync::CancellationToken;

use super::{Scheduler, TaskExecutor, TaskFailure, TaskPoolSize};
use crate::task::{Task, TaskKind};
use crate::task_queue::TaskQueue;

/// Reports every execution. Library IDs pick the behavior: `fail` fails permanently, `panic`
/// panics, `busy` always fails retryably, `flaky` fails retryably twice, `slow…` reports its
/// start, then waits for a permit from `gate`, and `stoppable` reports its start, then stops
/// when the scheduler shuts down.
struct Recorder {
    executed: mpsc::UnboundedSender<String>,
    runs: Mutex<HashMap<String, u32>>,
    gate: Arc<Semaphore>,
}

impl TaskExecutor for Recorder {
    async fn execute(&self, task: Task, stop: CancellationToken) -> Result<(), TaskFailure> {
        let TaskKind::EmptyTrash { library_id } = task.kind else {
            panic!("The recorder only receives empty-trash tasks, got {task:?}");
        };
        let library = library_id.to_string();
        let run = {
            let mut runs = self.runs.lock().expect("Runs lock is not poisoned");
            let run = runs.entry(library.clone()).or_default();
            *run += 1;
            *run
        };
        if library == "stoppable" {
            self.executed
                .send(format!("{library} started"))
                .expect("Test receiver is alive");
            stop.cancelled().await;
            return Err(TaskFailure::Interrupted);
        }
        if library.starts_with("slow") {
            self.executed
                .send(format!("{library} started"))
                .expect("Test receiver is alive");
            self.gate.acquire().await.expect("Gate stays open").forget();
        }
        self.executed
            .send(library.clone())
            .expect("Test receiver is alive");
        match library.as_str() {
            "fail" => Err(TaskFailure::Permanent("failed on purpose".to_owned())),
            "panic" => panic!("Panics on purpose"),
            "busy" => Err(TaskFailure::Retryable("busy".to_owned())),
            "flaky" if run < 3 => Err(TaskFailure::Retryable("flaky".to_owned())),
            _ => Ok(()),
        }
    }
}

async fn queue(directory: &Path) -> TaskQueue {
    let db = Database::connect(&directory.join("tasks.sqlite"))
        .await
        .expect("Open tasks database");
    db.migrate_tasks().await.expect("Migrate tasks database");
    TaskQueue::new(db)
}

async fn start(
    queue: &TaskQueue,
    size: &TaskPoolSize,
) -> (Scheduler, mpsc::UnboundedReceiver<String>, Arc<Semaphore>) {
    let (executed, receiver) = mpsc::unbounded_channel();
    let gate = Arc::new(Semaphore::new(0));
    let recorder = Recorder {
        executed,
        runs: Mutex::default(),
        gate: Arc::clone(&gate),
    };
    let scheduler = Scheduler::start(queue.clone(), recorder, size)
        .await
        .expect("Start scheduler");
    (scheduler, receiver, gate)
}

async fn next(receiver: &mut mpsc::UnboundedReceiver<String>) -> String {
    time::timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("A task runs within ten seconds")
        .expect("Executor is alive")
}

async fn queued(directory: &Path) -> i64 {
    let db = Database::connect(&directory.join("tasks.sqlite"))
        .await
        .expect("Open tasks database");
    let mut tx = db.begin().await.expect("Begin");
    TasksDao::count(&mut tx).await.expect("Count tasks")
}

#[tokio::test]
async fn tasks_committed_before_start_run_by_priority_after_deduplication() {
    let directory = komga_test_support::tempdir().expect("Tasks directory");
    let queue = queue(directory.path()).await;
    for task in [
        Task::empty_trash("a".into()),
        Task::empty_trash("b".into()),
        Task::empty_trash("a".into()).high(),
    ] {
        queue.submit(&task).await.expect("Submit");
    }
    // A new queue over the same file: nothing but the committed rows carries over.
    let (scheduler, mut executed, _) =
        start(&self::queue(directory.path()).await, &TaskPoolSize::new(1)).await;
    assert_eq!(
        [next(&mut executed).await, next(&mut executed).await],
        ["a", "b"]
    );
    scheduler.shutdown().await;
    assert!(executed.try_recv().is_err(), "the duplicate ran once");
    assert_eq!(queued(directory.path()).await, 0);
}

#[tokio::test]
async fn idle_workers_wake_on_submission_and_keep_permanent_failures() {
    let directory = komga_test_support::tempdir().expect("Tasks directory");
    let queue = queue(directory.path()).await;
    let (scheduler, mut executed, _) = start(&queue, &TaskPoolSize::new(1)).await;
    for library in ["fail", "panic", "ok"] {
        queue
            .submit(&Task::empty_trash(library.into()))
            .await
            .expect("Submit");
        assert_eq!(next(&mut executed).await, library);
    }
    scheduler.shutdown().await;
    assert_eq!(
        queued(directory.path()).await,
        2,
        "failed tasks stay recorded and are not retried"
    );
    assert!(executed.try_recv().is_err());
    let stats = queue.metrics().snapshot()["EmptyTrash"];
    assert_eq!((stats.executions, stats.failures), (1, 2), "{stats:?}");
    assert_eq!(stats.total_time, stats.max_time);
}

#[tokio::test]
async fn retryable_failures_back_off_until_success_or_the_attempt_budget() {
    let directory = komga_test_support::tempdir().expect("Tasks directory");
    let queue = queue(directory.path()).await;
    let (scheduler, mut executed, _) = start(&queue, &TaskPoolSize::new(1)).await;
    let started = Instant::now();
    for library in ["flaky", "busy"] {
        queue
            .submit(&Task::empty_trash(library.into()))
            .await
            .expect("Submit");
    }
    let mut runs = Vec::new();
    for _ in 0..6 {
        runs.push(next(&mut executed).await);
    }
    runs.sort();
    assert_eq!(runs, ["busy", "busy", "busy", "flaky", "flaky", "flaky"]);
    assert!(
        started.elapsed() >= Duration::from_secs(3),
        "1 s and 2 s backoffs"
    );
    time::sleep(Duration::from_millis(200)).await;
    scheduler.shutdown().await;
    assert!(executed.try_recv().is_err(), "no fourth attempt");
    assert_eq!(
        queued(directory.path()).await,
        1,
        "only the exhausted task stays"
    );
}

#[tokio::test]
async fn a_task_resubmitted_while_running_runs_again() {
    let directory = komga_test_support::tempdir().expect("Tasks directory");
    let queue = queue(directory.path()).await;
    let (scheduler, mut executed, gate) = start(&queue, &TaskPoolSize::new(1)).await;
    let slow = Task::empty_trash("slow".into());
    queue.submit(&slow).await.expect("Submit");
    assert_eq!(next(&mut executed).await, "slow started");
    queue.submit(&slow).await.expect("Resubmit while running");
    gate.add_permits(2);
    let mut runs = Vec::new();
    for _ in 0..3 {
        runs.push(next(&mut executed).await);
    }
    assert_eq!(runs, ["slow", "slow started", "slow"]);
    scheduler.shutdown().await;
    assert_eq!(queued(directory.path()).await, 0);
}

#[tokio::test]
async fn pool_size_changes_apply_without_interrupting_running_tasks() {
    let directory = komga_test_support::tempdir().expect("Tasks directory");
    let queue = queue(directory.path()).await;
    let size = TaskPoolSize::new(1);
    let (scheduler, mut executed, gate) = start(&queue, &size).await;
    for library in ["slow1", "slow2"] {
        queue
            .submit(&Task::empty_trash(library.into()))
            .await
            .expect("Submit");
    }
    assert_eq!(next(&mut executed).await, "slow1 started");
    time::sleep(Duration::from_millis(200)).await;
    assert!(executed.try_recv().is_err(), "one worker runs one task");
    size.set(2);
    assert_eq!(next(&mut executed).await, "slow2 started");

    size.set(1);
    for library in ["slow3", "slow4"] {
        queue
            .submit(&Task::empty_trash(library.into()))
            .await
            .expect("Submit");
    }
    gate.add_permits(2);
    let mut runs = Vec::new();
    for _ in 0..3 {
        runs.push(next(&mut executed).await);
    }
    runs.sort();
    assert_eq!(&runs[..2], ["slow1", "slow2"], "running tasks finish");
    assert!(runs[2].ends_with(" started"), "{runs:?}");
    time::sleep(Duration::from_millis(200)).await;
    assert!(
        executed.try_recv().is_err(),
        "after shrinking, one worker runs one task"
    );
    gate.add_permits(2);
    for _ in 0..3 {
        next(&mut executed).await;
    }
    scheduler.shutdown().await;
    assert_eq!(queued(directory.path()).await, 0);
}

#[tokio::test]
async fn a_shutdown_stops_running_tasks_that_stop_and_queues_them_again() {
    let directory = komga_test_support::tempdir().expect("Tasks directory");
    let queue = queue(directory.path()).await;
    let (scheduler, mut executed, _) = start(&queue, &TaskPoolSize::new(1)).await;
    queue
        .submit(&Task::empty_trash("stoppable".into()))
        .await
        .expect("Submit");
    assert_eq!(next(&mut executed).await, "stoppable started");
    time::timeout(Duration::from_secs(10), scheduler.shutdown())
        .await
        .expect("The running task stops");
    assert_eq!(queued(directory.path()).await, 1);
    assert!(
        queue.metrics().snapshot().is_empty(),
        "neither a success nor a failure"
    );

    let (scheduler, mut executed, _) = start(&queue, &TaskPoolSize::new(1)).await;
    assert_eq!(
        next(&mut executed).await,
        "stoppable started",
        "runs again on restart"
    );
    scheduler.shutdown().await;
}

#[test]
fn task_end_backoff_does_not_overflow_on_unexpected_attempt_counts() {
    let retryable = Err(TaskFailure::Retryable("busy".to_owned()));
    assert_eq!(
        super::task_end(0, &retryable),
        komga_db::TaskEnd::Retry {
            delay_seconds: 1,
            failure: "busy",
        }
    );
    assert_eq!(
        super::task_end(-5, &retryable),
        komga_db::TaskEnd::Retry {
            delay_seconds: 1,
            failure: "busy",
        }
    );
    assert_eq!(
        super::task_end(1, &retryable),
        komga_db::TaskEnd::Retry {
            delay_seconds: 1,
            failure: "busy",
        }
    );
    assert_eq!(
        super::task_end(2, &retryable),
        komga_db::TaskEnd::Retry {
            delay_seconds: 2,
            failure: "busy",
        }
    );
}
