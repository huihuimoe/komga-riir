//! Kotlin `TaskHandler`'s `komga.tasks.execution` timer and `komga.tasks.failure` counter.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// Executions since startup, by task type.
#[derive(Clone, Default)]
pub struct TaskMetrics(Arc<Mutex<BTreeMap<&'static str, TaskStats>>>);

/// Succeeded executions are timed; failed ones are only counted.
#[derive(Debug, Clone, Copy, Default)]
pub struct TaskStats {
    pub executions: u64,
    pub total_time: Duration,
    pub max_time: Duration,
    pub failures: u64,
}

impl TaskMetrics {
    #[must_use]
    pub fn snapshot(&self) -> BTreeMap<&'static str, TaskStats> {
        self.stats().clone()
    }

    pub(crate) fn succeeded(&self, task_type: &'static str, time: Duration) {
        let mut stats = self.stats();
        let stats = stats.entry(task_type).or_default();
        stats.executions += 1;
        stats.total_time += time;
        stats.max_time = stats.max_time.max(time);
    }

    pub(crate) fn failed(&self, task_type: &'static str) {
        self.stats().entry(task_type).or_default().failures += 1;
    }

    fn stats(&self) -> std::sync::MutexGuard<'_, BTreeMap<&'static str, TaskStats>> {
        // The map stays consistent: each update is a few field writes that cannot panic.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
