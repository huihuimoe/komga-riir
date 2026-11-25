//! The Prometheus exposition: the HTTP meters the API records, the task meters, and
//! zero-overhead Tokio runtime snapshots.

use std::collections::BTreeMap;
use std::fmt::{Display, Write as _};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use komga_db::DbError;
use komga_scheduler::TaskStats;
use tokio::runtime::Handle;

use super::MetricsService;

const HTTP_BUCKETS: [f64; 11] = [
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Histograms keyed by route template and HTTP status code.
type HistogramsByUri = BTreeMap<String, BTreeMap<u16, Histogram>>;

/// Request times by method, route template and status; nested so lookups borrow `&str` without
/// allocating on every request while preserving lexicographic `(method, uri, status)` order.
#[derive(Default)]
pub(super) struct HttpRequests(Mutex<BTreeMap<&'static str, HistogramsByUri>>);

/// A task meter family: its name, type and value per task type.
type TaskFamily = (&'static str, &'static str, fn(&TaskStats) -> f64);

/// Each bucket counts the requests up to its bound, as the format's `le` buckets do.
#[derive(Default)]
struct Histogram {
    buckets: [u64; HTTP_BUCKETS.len()],
    count: u64,
    sum: f64,
}

impl MetricsService {
    pub fn record_request(&self, method: &'static str, uri: &str, status: u16, time: Duration) {
        let seconds = time.as_secs_f64();
        let mut requests = self
            .0
            .http_requests
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let by_uri = requests.entry(method).or_default();
        let by_status = match by_uri.get_mut(uri) {
            Some(by_status) => by_status,
            None => by_uri.entry(uri.to_owned()).or_default(),
        };
        let histogram = by_status.entry(status).or_default();
        for (bucket, bound) in histogram.buckets.iter_mut().zip(HTTP_BUCKETS) {
            *bucket += u64::from(seconds <= bound);
        }
        histogram.count += 1;
        histogram.sum += seconds;
    }

    /// The task meters are read from the queue at each scrape, so the queue stays their
    /// only source.
    pub async fn scrape(&self) -> Result<String, DbError> {
        let queued: i64 = self.0.tasks.count_by_type().await?.values().sum();
        let mut text = String::new();
        self.write_http_requests(&mut text);
        writeln!(
            text,
            "# TYPE komga_tasks_queued gauge\nkomga_tasks_queued {queued}"
        )
        .ok();
        let tasks = self.0.tasks.metrics().snapshot();
        let families: [TaskFamily; 3] = [
            ("komga_tasks_executions_total", "counter", |stats| {
                stats.executions as f64
            }),
            ("komga_tasks_execution_seconds_total", "gauge", |stats| {
                stats.total_time.as_secs_f64()
            }),
            ("komga_tasks_failures_total", "counter", |stats| {
                stats.failures as f64
            }),
        ];
        for (name, kind, value) in families {
            writeln!(text, "# TYPE {name} {kind}").ok();
            for (task_type, stats) in &tasks {
                writeln!(text, r#"{name}{{type="{task_type}"}} {}"#, value(stats)).ok();
            }
        }
        write_tokio_runtime(&mut text);
        Ok(text)
    }

    fn write_http_requests(&self, text: &mut String) {
        let name = "http_server_requests_seconds";
        writeln!(text, "# TYPE {name} histogram").ok();
        let requests = self
            .0
            .http_requests
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for (method, by_uri) in requests.iter() {
            for (uri, by_status) in by_uri {
                for (status, histogram) in by_status {
                    let labels = format!(r#"method="{method}",uri="{uri}",status="{status}""#);
                    for (count, bound) in histogram.buckets.iter().zip(HTTP_BUCKETS) {
                        writeln!(text, r#"{name}_bucket{{{labels},le="{bound}"}} {count}"#).ok();
                    }
                    let count = histogram.count;
                    writeln!(text, r#"{name}_bucket{{{labels},le="+Inf"}} {count}"#).ok();
                    writeln!(text, "{name}_sum{{{labels}}} {}", histogram.sum).ok();
                    writeln!(text, "{name}_count{{{labels}}} {count}").ok();
                }
            }
        }
    }
}

/// Aggregates runtime-wide Tokio snapshots without per-worker labels to keep cardinality fixed.
fn write_tokio_runtime(text: &mut String) {
    let metrics = Handle::current().metrics();
    let workers = metrics.num_workers();

    let mut local_queue_depth = 0_usize;
    let mut busy_duration = Duration::ZERO;
    let mut polls = 0_u64;
    let mut parks = 0_u64;
    let mut steals = 0_u64;
    let mut overflows = 0_u64;
    for worker in 0..workers {
        local_queue_depth += metrics.worker_local_queue_depth(worker);
        busy_duration += metrics.worker_total_busy_duration(worker);
        polls += metrics.worker_poll_count(worker);
        parks += metrics.worker_park_count(worker);
        steals += metrics.worker_steal_count(worker);
        overflows += metrics.worker_overflow_count(worker);
    }

    write_sample(text, "tokio_workers_count", "gauge", workers);
    write_sample(
        text,
        "tokio_alive_tasks",
        "gauge",
        metrics.num_alive_tasks(),
    );
    write_sample(
        text,
        "tokio_spawned_tasks_total",
        "counter",
        metrics.spawned_tasks_count(),
    );
    write_sample(
        text,
        "tokio_global_queue_depth",
        "gauge",
        metrics.global_queue_depth(),
    );
    write_sample(text, "tokio_local_queue_depth", "gauge", local_queue_depth);
    write_sample(
        text,
        "tokio_budget_forced_yields_total",
        "counter",
        metrics.budget_forced_yield_count(),
    );
    write_sample(
        text,
        "tokio_worker_busy_seconds_total",
        "counter",
        busy_duration.as_secs_f64(),
    );
    write_sample(text, "tokio_worker_polls_total", "counter", polls);
    write_sample(text, "tokio_worker_parks_total", "counter", parks);
    write_sample(text, "tokio_worker_steals_total", "counter", steals);
    write_sample(text, "tokio_worker_overflows_total", "counter", overflows);
    write_sample(
        text,
        "tokio_blocking_threads",
        "gauge",
        metrics.num_blocking_threads(),
    );
    write_sample(
        text,
        "tokio_idle_blocking_threads",
        "gauge",
        metrics.num_idle_blocking_threads(),
    );
    write_sample(
        text,
        "tokio_blocking_queue_depth",
        "gauge",
        metrics.blocking_queue_depth(),
    );
    write_sample(
        text,
        "tokio_io_driver_fd_registered_total",
        "counter",
        metrics.io_driver_fd_registered_count(),
    );
    write_sample(
        text,
        "tokio_io_driver_fd_deregistered_total",
        "counter",
        metrics.io_driver_fd_deregistered_count(),
    );
    write_sample(
        text,
        "tokio_io_driver_ready_total",
        "counter",
        metrics.io_driver_ready_count(),
    );
}

fn write_sample(text: &mut String, name: &str, kind: &str, value: impl Display) {
    writeln!(text, "# TYPE {name} {kind}\n{name} {value}").ok();
}
