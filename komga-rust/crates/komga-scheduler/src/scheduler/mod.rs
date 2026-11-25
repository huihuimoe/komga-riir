//! Kotlin `TaskProcessor`: workers that claim queued tasks and hand them to the executor.

use std::future::Future;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use komga_db::{DbError, TaskEnd};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::metrics::TaskMetrics;
use crate::task::Task;
use crate::task_queue::{MAX_ATTEMPTS, TaskQueue};

/// Delay before polling again after a tasks database failure.
const RETRY_DELAY: Duration = Duration::from_secs(1);
/// Initial backoff when persisting a task's completion fails.
const FINISH_RETRY_INITIAL: Duration = Duration::from_millis(100);
/// Upper bound on backoff between finish persistence retries.
const FINISH_RETRY_MAX: Duration = Duration::from_secs(2);
/// Maximum finish persistence attempts once shutdown has been requested.
const FINISH_SHUTDOWN_ATTEMPTS: u32 = 10;

/// Business execution of a claimed task, implemented by the service layer.
pub trait TaskExecutor: Send + Sync + 'static {
    /// `stop` is cancelled when the scheduler shuts down. Long tasks check it at points where
    /// stopping leaves nothing half-done, and return [`TaskFailure::Interrupted`] there.
    fn execute(
        &self,
        task: Task,
        stop: CancellationToken,
    ) -> impl Future<Output = Result<(), TaskFailure>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum TaskFailure {
    /// A temporary failure, such as a busy or unavailable resource; retried with backoff.
    #[error("{0}")]
    Retryable(String),
    /// A failure that repeating cannot fix; the task stays failed until submitted again.
    #[error("{0}")]
    Permanent(String),
    /// Stopped for a shutdown; the task is queued again without using an attempt.
    #[error("Interrupted by a shutdown")]
    Interrupted,
}

/// Kotlin `SettingChangedEvent.TaskPoolSize`: how many tasks may run at once. Changes apply
/// immediately; shrinking lets running tasks finish.
#[derive(Clone)]
pub struct TaskPoolSize(watch::Sender<NonZeroUsize>);

impl TaskPoolSize {
    /// The stored `taskPoolSize`; values below 1 run one worker, like Kotlin's executor.
    #[must_use]
    pub fn new(setting: i32) -> Self {
        Self(watch::Sender::new(Self::workers(setting)))
    }

    pub fn set(&self, setting: i32) {
        self.0.send_replace(Self::workers(setting));
    }

    fn workers(setting: i32) -> NonZeroUsize {
        usize::try_from(setting)
            .ok()
            .and_then(NonZeroUsize::new)
            .unwrap_or(NonZeroUsize::MIN)
    }
}

pub struct Scheduler {
    stop: CancellationToken,
    supervisor: JoinHandle<()>,
}

impl Scheduler {
    /// Releases tasks a previous process left running, then starts `size` workers; each first
    /// drains tasks already queued.
    ///
    /// # Errors
    /// Returns the tasks database failure from releasing interrupted tasks.
    pub async fn start<E: TaskExecutor>(
        queue: TaskQueue,
        executor: E,
        size: &TaskPoolSize,
    ) -> Result<Self, DbError> {
        queue.disown().await?;
        let stop = CancellationToken::new();
        let supervisor = tokio::spawn(supervise(
            queue,
            Arc::new(executor),
            size.0.subscribe(),
            stop.clone(),
        ));
        Ok(Self { stop, supervisor })
    }

    /// Stops claiming tasks, asks running tasks to stop at their next safe point, and waits
    /// for them.
    pub async fn shutdown(self) {
        self.stop.cancel();
        if let Err(error) = self.supervisor.await {
            error!(%error, "Task supervisor failed");
        }
    }
}

/// Spawns workers as the pool grows. Workers above a smaller size park instead of exiting, so
/// worker `n` is always `taskProcessor-n`.
async fn supervise<E: TaskExecutor>(
    queue: TaskQueue,
    executor: Arc<E>,
    mut size: watch::Receiver<NonZeroUsize>,
    stop: CancellationToken,
) {
    let mut workers = JoinSet::new();
    loop {
        let target = size.borrow_and_update().get();
        for index in workers.len() + 1..=target {
            workers.spawn(work(
                queue.clone(),
                Arc::clone(&executor),
                index,
                size.clone(),
                stop.clone(),
            ));
        }
        tokio::select! {
            Ok(()) = size.changed() => {}
            () = stop.cancelled() => break,
        }
    }
    while let Some(result) = workers.join_next().await {
        if let Err(error) = result {
            error!(%error, "Task worker failed");
        }
    }
}

async fn work<E: TaskExecutor>(
    queue: TaskQueue,
    executor: Arc<E>,
    index: usize,
    mut size: watch::Receiver<NonZeroUsize>,
    stop: CancellationToken,
) {
    let owner = format!("taskProcessor-{index}");
    loop {
        if stop.is_cancelled() {
            return;
        }
        // Park before subscribing to `wake` so `notify_one` never spends a wakeup on an idle
        // worker above the configured pool size.
        if size.borrow().get() < index {
            tokio::select! {
                Ok(_) = size.wait_for(|size| size.get() >= index) => continue,
                () = stop.cancelled() => return,
            }
        }
        // Register for wake-ups before polling so a submission in between is not missed.
        let wake = queue.wake().notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        match queue.take(&owner).await {
            Ok(Some(claimed)) => {
                let result = match &claimed.task {
                    Ok(task) => run(&executor, task.clone(), &stop, queue.metrics()).await,
                    Err(error) => {
                        error!(task = claimed.id, %error, "Could not deserialize the task");
                        Err(TaskFailure::Permanent(error.to_string()))
                    }
                };
                let end = task_end(claimed.attempts, &result);
                finish_with_retry(&queue, &claimed, end, &stop).await;
            }
            Ok(None) => {
                let retry = queue.next_retry().await.unwrap_or_else(|error| {
                    error!(%error, "Could not read the next retry time");
                    Some(RETRY_DELAY)
                });
                tokio::select! {
                    () = &mut wake => {}
                    () = sleep_for(retry) => {}
                    () = stop.cancelled() => return,
                }
            }
            Err(error) => {
                error!(%error, "Could not take a task from the queue");
                tokio::select! {
                    () = time::sleep(RETRY_DELAY) => {}
                    () = stop.cancelled() => return,
                }
            }
        }
    }
}

async fn sleep_for(duration: Option<Duration>) {
    match duration {
        Some(duration) => time::sleep(duration).await,
        None => std::future::pending().await,
    }
}

/// Persists a claimed task's outcome, retrying transient DB errors so the task and its group
/// cannot stay stuck as owned until restart.
async fn finish_with_retry(
    queue: &TaskQueue,
    claimed: &crate::task_queue::Claimed,
    end: TaskEnd<'_>,
    stop: &CancellationToken,
) {
    let mut delay = FINISH_RETRY_INITIAL;
    let mut shutdown_attempts = 0_u32;
    loop {
        match queue.finish(claimed, end).await {
            Ok(()) => return,
            Err(error) => {
                error!(task = claimed.id, %error, "Could not record the task result, retrying");
                if stop.is_cancelled() {
                    shutdown_attempts += 1;
                    if shutdown_attempts >= FINISH_SHUTDOWN_ATTEMPTS {
                        error!(
                            task = claimed.id,
                            attempts = shutdown_attempts,
                            "Giving up recording task result during shutdown"
                        );
                        return;
                    }
                }
                time::sleep(delay).await;
                delay = (delay * 2).min(FINISH_RETRY_MAX);
            }
        }
    }
}

/// Retryable failures back off 1 s, 2 s, … until `MAX_ATTEMPTS` executions have failed.
fn task_end(attempts: i32, result: &Result<(), TaskFailure>) -> TaskEnd<'_> {
    match result {
        Ok(()) => TaskEnd::Succeeded,
        Err(TaskFailure::Interrupted) => TaskEnd::Interrupted,
        Err(TaskFailure::Retryable(failure)) if attempts < MAX_ATTEMPTS => {
            let exponent = u32::try_from(attempts.saturating_sub(1))
                .unwrap_or(0)
                .min(30);
            TaskEnd::Retry {
                delay_seconds: 1_i64 << exponent,
                failure,
            }
        }
        Err(TaskFailure::Retryable(failure) | TaskFailure::Permanent(failure)) => {
            TaskEnd::Failed { failure }
        }
    }
}

async fn run<E: TaskExecutor>(
    executor: &Arc<E>,
    task: Task,
    stop: &CancellationToken,
    metrics: &TaskMetrics,
) -> Result<(), TaskFailure> {
    let description = format!("{task:?}");
    let task_type = task.simple_type();
    info!(task = description, "Executing task");
    let started = Instant::now();
    let executor = Arc::clone(executor);
    let stop = stop.clone();
    // A separate Tokio task turns a panic into a failure instead of killing the worker.
    let result = tokio::spawn(async move { executor.execute(task, stop).await })
        .await
        .unwrap_or_else(|error| Err(TaskFailure::Permanent(error.to_string())));
    match &result {
        Ok(()) => {
            let elapsed = started.elapsed();
            info!(task = description, ?elapsed, "Task executed");
            metrics.succeeded(task_type, elapsed);
        }
        Err(TaskFailure::Interrupted) => info!(task = description, "Task interrupted"),
        Err(failure) => {
            error!(task = description, %failure, "Task execution failed");
            metrics.failed(task_type);
        }
    }
    result
}

#[cfg(test)]
mod tests;
