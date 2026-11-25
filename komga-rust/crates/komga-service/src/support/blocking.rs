//! Bounded pool for blocking filesystem calls made on behalf of HTTP requests, so slow or hung
//! mounts cannot pile up threads; requests that wait too long for a slot fail instead.

use std::time::Duration;

use thiserror::Error;
use tokio::sync::Semaphore;
use tokio::task::{self, JoinError};
use tokio::time;

static FOREGROUND: BlockingPool = BlockingPool::new(16, Duration::from_secs(5));

#[derive(Debug, Error)]
pub enum BlockingError {
    #[error("Filesystem workers are busy")]
    Saturated,
    #[error("Filesystem worker failed: {0}")]
    Worker(#[from] JoinError),
}

/// Runs `job` on the foreground pool.
pub(crate) async fn run<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, BlockingError> {
    FOREGROUND.run(job).await
}

struct BlockingPool {
    permits: Semaphore,
    deadline: Duration,
}

impl BlockingPool {
    const fn new(capacity: usize, deadline: Duration) -> Self {
        Self {
            permits: Semaphore::const_new(capacity),
            deadline,
        }
    }

    async fn run<T: Send + 'static>(
        &'static self,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, BlockingError> {
        let permit = time::timeout(self.deadline, self.permits.acquire())
            .await
            .map_err(|_| BlockingError::Saturated)?
            .expect("The pool semaphore is never closed");
        // The job owns its slot: a call that outlives a cancelled request still counts.
        let result = task::spawn_blocking(move || {
            let _permit = permit;
            job()
        })
        .await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use tokio::time;

    use super::{BlockingError, BlockingPool};

    #[tokio::test]
    async fn a_saturated_pool_fails_within_the_deadline_and_recovers_after_release() {
        let pool: &'static BlockingPool =
            Box::leak(Box::new(BlockingPool::new(1, Duration::from_millis(100))));
        let (release, released) = mpsc::channel::<()>();
        let held = tokio::spawn(pool.run(move || released.recv().is_ok()));
        time::timeout(Duration::from_secs(1), async {
            while pool.permits.available_permits() > 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("The first job holds the only slot while it runs");
        assert!(matches!(
            pool.run(|| ()).await,
            Err(BlockingError::Saturated)
        ));
        release.send(()).expect("The held job is waiting");
        assert!(held.await.expect("Held job").expect("Held job runs"));
        assert_eq!(pool.run(|| 7).await.expect("A released slot is reused"), 7);
    }
}
