//! Kotlin `PeriodicScannerController` and `LibraryScanScheduler`: libraries are scanned at startup
//! when they ask for it, then every scan interval.

use std::collections::HashMap;
use std::future;
use std::time::Duration;

use komga_service::{EventBus, LibraryService};
use komga_types::{DomainEvent, LibraryId};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio::time::{self, Instant};
use tracing::{error, info};

/// Queues the scans until the returned task is aborted.
pub(super) fn spawn(libraries: LibraryService, events: &EventBus) -> JoinHandle<()> {
    // Subscribed before the libraries are first read, so that no change is missed.
    tokio::spawn(run(libraries, events.subscribe()))
}

async fn run(libraries: LibraryService, mut events: Receiver<DomainEvent>) {
    if let Err(error) = libraries.scan_on_startup().await {
        error!(error = %error, "Failed to queue the scans on startup");
    }
    let mut schedule = ScanSchedule::default();
    refresh(&libraries, &mut schedule).await;
    loop {
        let next = schedule.next_due();
        tokio::select! {
            received = events.recv() => match received {
                Ok(
                    DomainEvent::LibraryAdded(_)
                    | DomainEvent::LibraryUpdated(_)
                    | DomainEvent::LibraryDeleted(_),
                )
                | Err(RecvError::Lagged(_)) => refresh(&libraries, &mut schedule).await,
                Ok(_) => {}
                Err(RecvError::Closed) => return,
            },
            () = sleep_until(next) => {
                for library in schedule.take_due(Instant::now()) {
                    info!(%library, "Periodic scan");
                    if let Err(error) = libraries.scan_periodically(&library).await {
                        error!(%library, error = %error, "Failed to queue the periodic scan");
                    }
                }
            }
        }
    }
}

/// Keeps the previous schedule when the libraries cannot be read; the next library change retries.
async fn refresh(libraries: &LibraryService, schedule: &mut ScanSchedule) {
    match libraries.scan_periods().await {
        Ok(periods) => schedule.update(periods, Instant::now()),
        Err(error) => error!(error = %error, "Failed to read the library scan intervals"),
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => time::sleep_until(deadline).await,
        None => future::pending().await,
    }
}

#[derive(Default)]
struct ScanSchedule {
    libraries: HashMap<LibraryId, Slot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    period: Duration,
    next: Instant,
}

impl ScanSchedule {
    /// Like Kotlin, a library first scans one period after it is scheduled, and a changed period
    /// starts over. Unlike Kotlin, libraries added or deleted since startup are followed too.
    fn update(&mut self, periods: HashMap<LibraryId, Duration>, now: Instant) {
        self.libraries = periods
            .into_iter()
            .map(|(library, period)| {
                let slot = match self.libraries.remove(&library) {
                    Some(slot) if slot.period == period => slot,
                    _ => Slot {
                        period,
                        next: now + period,
                    },
                };
                (library, slot)
            })
            .collect();
    }

    fn next_due(&self) -> Option<Instant> {
        self.libraries.values().map(|slot| slot.next).min()
    }

    /// The libraries due at `now`, each once however many periods it missed; their next scan
    /// keeps the original phase.
    fn take_due(&mut self, now: Instant) -> Vec<LibraryId> {
        let mut due = Vec::new();
        for (library, slot) in &mut self.libraries {
            if slot.next <= now {
                due.push(library.clone());
                while slot.next <= now {
                    slot.next += slot.period;
                }
            }
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use komga_types::LibraryId;
    use tokio::time::Instant;

    use super::ScanSchedule;

    const HOUR: Duration = Duration::from_secs(60 * 60);

    fn periods(entries: &[(&str, Duration)]) -> HashMap<LibraryId, Duration> {
        entries
            .iter()
            .map(|&(library, period)| (LibraryId::from(library), period))
            .collect()
    }

    #[test]
    fn libraries_scan_one_period_after_scheduling_and_once_per_period() {
        let start = Instant::now();
        let mut schedule = ScanSchedule::default();
        schedule.update(periods(&[("hourly", HOUR), ("daily", 24 * HOUR)]), start);

        assert_eq!(schedule.next_due(), Some(start + HOUR));
        assert!(schedule.take_due(start + HOUR / 2).is_empty());
        assert_eq!(schedule.take_due(start + HOUR), [LibraryId::from("hourly")]);
        assert!(
            schedule.take_due(start + HOUR).is_empty(),
            "A period triggers its scan once"
        );
        assert_eq!(schedule.next_due(), Some(start + 2 * HOUR));
    }

    #[test]
    fn missed_periods_merge_into_one_scan_and_keep_the_phase() {
        let start = Instant::now();
        let mut schedule = ScanSchedule::default();
        schedule.update(periods(&[("hourly", HOUR)]), start);

        let late = start + 5 * HOUR + HOUR / 2;
        assert_eq!(schedule.take_due(late), [LibraryId::from("hourly")]);
        assert_eq!(schedule.next_due(), Some(start + 6 * HOUR));
    }

    #[test]
    fn updates_reschedule_only_changed_periods_and_follow_added_and_deleted_libraries() {
        let start = Instant::now();
        let mut schedule = ScanSchedule::default();
        schedule.update(
            periods(&[("kept", HOUR), ("changed", HOUR), ("deleted", HOUR)]),
            start,
        );

        let later = start + HOUR / 2;
        schedule.update(
            periods(&[("kept", HOUR), ("changed", 6 * HOUR), ("added", HOUR)]),
            later,
        );

        let mut due = schedule.take_due(start + HOUR);
        due.sort();
        assert_eq!(due, [LibraryId::from("kept")]);
        let mut due = schedule.take_due(later + HOUR);
        due.sort();
        assert_eq!(due, [LibraryId::from("added")]);
        assert_eq!(schedule.take_due(later + 6 * HOUR).len(), 3);
    }

    #[test]
    fn disabled_scans_leave_nothing_due() {
        let mut schedule = ScanSchedule::default();
        schedule.update(periods(&[("hourly", HOUR)]), Instant::now());
        schedule.update(HashMap::new(), Instant::now());

        assert_eq!(schedule.next_due(), None);
    }
}
