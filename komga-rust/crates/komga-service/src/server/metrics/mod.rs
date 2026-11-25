//! The Komga meters of Kotlin `MetricsPublisherController` and `TaskHandler`, read the way
//! Spring Boot's `MetricsEndpoint` serves them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use komga_db::{Database, DbError, EntityCountDao, LibraryCount, TotalCount};
use komga_scheduler::TaskQueue;

mod prometheus;

/// Serves the meters; entity counts are read when asked for, so they follow every change.
#[derive(Clone)]
pub struct MetricsService(Arc<MetricsServiceInner>);

struct MetricsServiceInner {
    db: Database,
    tasks: TaskQueue,
    http_requests: prometheus::HttpRequests,
}

/// The meters of one name matching the requested tags, combined.
#[derive(Debug)]
pub struct Metric {
    pub name: &'static str,
    pub description: &'static str,
    pub base_unit: Option<&'static str>,
    pub measurements: Vec<(Statistic, f64)>,
    /// The other tags of the matching meters, with their values.
    pub available_tags: BTreeMap<&'static str, BTreeSet<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Statistic {
    Value,
    Count,
    TotalTime,
    Max,
}

struct Definition {
    name: &'static str,
    description: &'static str,
    base_unit: Option<&'static str>,
    source: Source,
}

#[derive(Clone, Copy)]
enum Source {
    Total(TotalCount),
    PerLibrary(LibraryCount),
    TaskExecution,
    TaskFailure,
}

const fn count(name: &'static str, description: &'static str, source: Source) -> Definition {
    Definition {
        name,
        description,
        base_unit: Some("count"),
        source,
    }
}

const DEFINITIONS: [Definition; 9] = [
    count(
        "komga.books",
        "The number of books",
        Source::PerLibrary(LibraryCount::Books),
    ),
    Definition {
        name: "komga.books.filesize",
        description: "The cumulated filesize of books",
        base_unit: Some("bytes"),
        source: Source::PerLibrary(LibraryCount::BookFileSize),
    },
    count(
        "komga.collections",
        "The number of collections",
        Source::Total(TotalCount::Collections),
    ),
    count(
        "komga.libraries",
        "The number of libraries",
        Source::Total(TotalCount::Libraries),
    ),
    count(
        "komga.readlists",
        "The number of readlists",
        Source::Total(TotalCount::ReadLists),
    ),
    count(
        "komga.series",
        "The number of series",
        Source::PerLibrary(LibraryCount::Series),
    ),
    count(
        "komga.sidecars",
        "The number of sidecars",
        Source::PerLibrary(LibraryCount::Sidecars),
    ),
    Definition {
        name: "komga.tasks.execution",
        description: "Task execution time",
        base_unit: Some("seconds"),
        source: Source::TaskExecution,
    },
    Definition {
        name: "komga.tasks.failure",
        description: "Count of failed tasks",
        base_unit: None,
        source: Source::TaskFailure,
    },
];

/// One time series: its tags and measurements.
struct Meter {
    tags: Vec<(&'static str, String)>,
    measurements: Vec<(Statistic, f64)>,
}

impl MetricsService {
    #[must_use]
    pub fn new(db: Database, tasks: TaskQueue) -> Self {
        Self(Arc::new(MetricsServiceInner {
            db,
            tasks,
            http_requests: prometheus::HttpRequests::default(),
        }))
    }

    pub fn names() -> impl Iterator<Item = &'static str> {
        DEFINITIONS.iter().map(|definition| definition.name)
    }

    /// `None` when the name is unknown or no meter has all `tags`.
    pub async fn metric(
        &self,
        name: &str,
        tags: &[(String, String)],
    ) -> Result<Option<Metric>, DbError> {
        let Some(definition) = DEFINITIONS
            .iter()
            .find(|definition| definition.name == name)
        else {
            return Ok(None);
        };
        let meters: Vec<Meter> = self
            .meters(definition.source)
            .await?
            .into_iter()
            .filter(|meter| tags.iter().all(|tag| meter.has(tag)))
            .collect();
        Ok(combine(&meters).map(|measurements| Metric {
            name: definition.name,
            description: definition.description,
            base_unit: definition.base_unit,
            measurements,
            available_tags: available_tags(&meters, tags),
        }))
    }

    async fn meters(&self, source: Source) -> Result<Vec<Meter>, DbError> {
        let meters = match source {
            Source::Total(count) => {
                let value = EntityCountDao::total(&self.0.db, count).await?;
                vec![Meter::untagged(vec![(Statistic::Value, value as f64)])]
            }
            Source::PerLibrary(count) => EntityCountDao::per_library(&self.0.db, count)
                .await?
                .into_iter()
                .map(|(library, value)| Meter {
                    tags: vec![("library", library.to_string())],
                    measurements: vec![(Statistic::Value, value as f64)],
                })
                .collect(),
            Source::TaskExecution => self.task_executions(),
            Source::TaskFailure => self.task_failures(),
        };
        Ok(meters)
    }

    /// Kotlin registers the timer untagged at startup, then one per task type on its first
    /// successful execution.
    fn task_executions(&self) -> Vec<Meter> {
        let typed = self
            .0
            .tasks
            .metrics()
            .snapshot()
            .into_iter()
            .filter(|(_, stats)| stats.executions > 0);
        let typed = typed.map(|(task_type, stats)| {
            Meter::task(
                task_type,
                timer_measurements(
                    stats.executions,
                    stats.total_time.as_secs_f64(),
                    stats.max_time.as_secs_f64(),
                ),
            )
        });
        std::iter::once(Meter::untagged(timer_measurements(0, 0.0, 0.0)))
            .chain(typed)
            .collect()
    }

    /// Like [`Self::task_executions`], on the first failure.
    fn task_failures(&self) -> Vec<Meter> {
        let typed = self
            .0
            .tasks
            .metrics()
            .snapshot()
            .into_iter()
            .filter(|(_, stats)| stats.failures > 0);
        let typed = typed.map(|(task_type, stats)| {
            Meter::task(task_type, vec![(Statistic::Count, stats.failures as f64)])
        });
        std::iter::once(Meter::untagged(vec![(Statistic::Count, 0.0)]))
            .chain(typed)
            .collect()
    }
}

impl Meter {
    fn untagged(measurements: Vec<(Statistic, f64)>) -> Self {
        Self {
            tags: Vec::new(),
            measurements,
        }
    }

    fn task(task_type: &str, measurements: Vec<(Statistic, f64)>) -> Self {
        Self {
            tags: vec![("type", task_type.to_owned())],
            measurements,
        }
    }

    fn has(&self, (key, value): &(String, String)) -> bool {
        self.tags.iter().any(|(k, v)| k == key && v == value)
    }
}

fn timer_measurements(count: u64, total_time: f64, max: f64) -> Vec<(Statistic, f64)> {
    vec![
        (Statistic::Count, count as f64),
        (Statistic::TotalTime, total_time),
        (Statistic::Max, max),
    ]
}

/// Spring sums each statistic across the meters, except `MAX` which takes the largest.
fn combine(meters: &[Meter]) -> Option<Vec<(Statistic, f64)>> {
    let (first, rest) = meters.split_first()?;
    let mut combined = first.measurements.clone();
    for meter in rest {
        for ((statistic, total), (_, value)) in combined.iter_mut().zip(&meter.measurements) {
            *total = if *statistic == Statistic::Max {
                total.max(*value)
            } else {
                *total + value
            };
        }
    }
    Some(combined)
}

fn available_tags(
    meters: &[Meter],
    requested: &[(String, String)],
) -> BTreeMap<&'static str, BTreeSet<String>> {
    let mut available: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for (key, value) in meters.iter().flat_map(|meter| &meter.tags) {
        if !requested.iter().any(|(requested, _)| requested == key) {
            available.entry(*key).or_default().insert(value.clone());
        }
    }
    available
}
