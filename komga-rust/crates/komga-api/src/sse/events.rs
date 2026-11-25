//! `GET /sse/v1/events`: committed changes the user may see, and the task queue for
//! administrators.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use komga_service::{EventFeed, SessionRegistry, TaskService};
use komga_types::{DomainEvent, KomgaUser, SetId, ThumbnailRef};
use serde::Serialize;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast, mpsc};
use tokio::time::{self, Interval, MissedTickBehavior};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::dto::{
    BookImportSseDto, BookSseDto, CollectionSseDto, LibrarySseDto, ReadListSseDto,
    ReadProgressSeriesSseDto, ReadProgressSseDto, SeriesSseDto, SessionExpiredSseDto,
    TaskQueueSseDto, ThumbnailBookSseDto, ThumbnailReadListSseDto, ThumbnailSeriesCollectionSseDto,
    ThumbnailSeriesSseDto,
};
use crate::auth::{RequestAuthContext, authenticate_request};
use crate::routes::ApiState;

const HEARTBEAT_PERIOD: Duration = Duration::from_secs(15);
const TASK_STATUS_PERIOD: Duration = Duration::from_secs(10);
/// Events a connection holds while its client reads slowly.
const BUFFER: usize = 16;
/// Open event streams, each holding a task and a subscription, beyond which new ones are refused.
const MAX_CONNECTIONS: usize = 1024;

/// Bounds the open event streams. Ends every one, and refuses new ones, once the server shuts
/// down: an open stream would otherwise hold graceful shutdown until it times out.
#[derive(Clone)]
pub struct SseConnections {
    closed: CancellationToken,
    slots: Arc<Semaphore>,
}

impl Default for SseConnections {
    fn default() -> Self {
        Self::with_limit(MAX_CONNECTIONS)
    }
}

impl SseConnections {
    fn with_limit(limit: usize) -> Self {
        Self {
            closed: CancellationToken::new(),
            slots: Arc::new(Semaphore::new(limit)),
        }
    }

    pub fn close(&self) {
        self.slots.close();
        self.closed.cancel();
    }

    /// A slot held until the stream is dropped; `None` when all are taken or once closed.
    fn open(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.slots).try_acquire_owned().ok()
    }
}

#[derive(Clone)]
struct SseState {
    events: EventFeed,
    tasks: TaskService,
    sessions: SessionRegistry,
    connections: SseConnections,
}

pub fn router(state: &ApiState) -> Router {
    let sessions = state.auth.sessions().clone();
    Router::new()
        .route("/sse/v1/events", get(subscribe))
        .route_layer(from_fn_with_state(state.auth.clone(), authenticate_request))
        .with_state(SseState {
            events: state.event_feed.clone(),
            tasks: state.tasks.clone(),
            sessions,
            connections: state.sse_connections.clone(),
        })
}

/// The user is taken as of the connection. The stream ends once the user's sessions are revoked,
/// which any change to what they may see does, or once its own session is logged out of.
async fn subscribe(
    Extension(auth): Extension<RequestAuthContext>,
    State(state): State<SseState>,
) -> Response {
    let Some(slot) = state.connections.open() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let closed = state.connections.closed.clone();
    let (sender, receiver) = mpsc::channel(BUFFER);
    // Subscribe before responding, so that no change committed after the response is missed.
    let subscription = state.events.subscribe();
    let logged_out = auth
        .session_token
        .as_deref()
        .map_or_else(CancellationToken::new, |token| {
            state.sessions.logged_out(token)
        });
    tokio::spawn(async move {
        let forwarding = forward(auth.user, subscription, state, sender);
        closed
            .run_until_cancelled(logged_out.run_until_cancelled(forwarding))
            .await;
    });
    // The slot is freed with the response body, which outlives forwarding while it drains.
    let stream = ReceiverStream::new(receiver).map(move |event| {
        let _slot = &slot;
        Ok::<Event, Infallible>(event)
    });
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(HEARTBEAT_PERIOD)
                .text("heartbeat"),
        )
        .into_response()
}

/// Sends the events `user` may see until the client goes away. A client too slow to keep up
/// loses its stream rather than some events, so that it reconnects and reloads.
async fn forward(
    user: Arc<KomgaUser>,
    mut events: broadcast::Receiver<DomainEvent>,
    state: SseState,
    sender: mpsc::Sender<Event>,
) {
    let mut task_status = user.is_admin().then(|| {
        let mut interval = time::interval(TASK_STATUS_PERIOD);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        interval
    });
    loop {
        let (event, last) = tokio::select! {
            () = sender.closed() => return,
            () = tick(task_status.as_mut()) => match task_queue_status(&state.tasks, &user).await {
                Some(event) => (event, false),
                None => continue,
            },
            received = events.recv() => match received {
                // Once their sessions are revoked, what the user may see is no longer `user`.
                Ok(domain_event) => match visible_event(&state.events, &user, &domain_event).await {
                    Some(event) => (event, matches!(domain_event, DomainEvent::SessionsExpired(_))),
                    None => continue,
                },
                Err(RecvError::Lagged(missed)) => {
                    warn!(user = %user.id, missed, "Closing an event stream that fell behind");
                    return;
                }
                Err(RecvError::Closed) => return,
            },
        };
        if sender.send(event).await.is_err() || last {
            return;
        }
    }
}

/// Waits for the next tick, or forever without an interval.
async fn tick(interval: Option<&mut Interval>) {
    match interval {
        Some(interval) => {
            interval.tick().await;
        }
        None => std::future::pending().await,
    }
}

/// Kotlin `emitTaskCount`, also sent as soon as an administrator connects.
async fn task_queue_status(tasks: &TaskService, user: &KomgaUser) -> Option<Event> {
    match tasks.queue_counts(user).await {
        Ok(count_by_type) => Some(event(
            "TaskQueueStatus",
            &TaskQueueSseDto {
                count: count_by_type.values().sum(),
                count_by_type,
            },
        )),
        Err(error) => {
            warn!(%error, "Failed to count queued tasks");
            None
        }
    }
}

/// Kotlin `handleSseEvent`, keeping only what `user` may see.
async fn visible_event(
    feed: &EventFeed,
    user: &KomgaUser,
    domain_event: &DomainEvent,
) -> Option<Event> {
    match feed.visible(user, domain_event).await {
        Ok(seen) => seen.as_ref().map(sse_event),
        Err(error) => {
            warn!(%error, "Failed to check whether a user may see an event");
            None
        }
    }
}

fn sse_event(domain_event: &DomainEvent) -> Event {
    match domain_event {
        DomainEvent::LibraryAdded(id) => event("LibraryAdded", &LibrarySseDto::from(id)),
        DomainEvent::LibraryUpdated(id) => event("LibraryChanged", &LibrarySseDto::from(id)),
        DomainEvent::LibraryDeleted(id) => event("LibraryDeleted", &LibrarySseDto::from(id)),
        DomainEvent::SeriesAdded(series) => event("SeriesAdded", &SeriesSseDto::from(series)),
        DomainEvent::SeriesUpdated(series) => event("SeriesChanged", &SeriesSseDto::from(series)),
        DomainEvent::SeriesDeleted(series) => event("SeriesDeleted", &SeriesSseDto::from(series)),
        DomainEvent::BookAdded(book) => event("BookAdded", &BookSseDto::from(book)),
        DomainEvent::BookUpdated(book) => event("BookChanged", &BookSseDto::from(book)),
        DomainEvent::BookDeleted(book) => event("BookDeleted", &BookSseDto::from(book)),
        DomainEvent::ReadListAdded(list) => event("ReadListAdded", &ReadListSseDto::from(list)),
        DomainEvent::ReadListUpdated(list) => event("ReadListChanged", &ReadListSseDto::from(list)),
        DomainEvent::ReadListDeleted(list) => event("ReadListDeleted", &ReadListSseDto::from(list)),
        DomainEvent::CollectionAdded(collection) => {
            event("CollectionAdded", &CollectionSseDto::from(collection))
        }
        DomainEvent::CollectionUpdated(collection) => {
            event("CollectionChanged", &CollectionSseDto::from(collection))
        }
        DomainEvent::CollectionDeleted(collection) => {
            event("CollectionDeleted", &CollectionSseDto::from(collection))
        }
        DomainEvent::ReadProgressChanged(progress) => {
            event("ReadProgressChanged", &ReadProgressSseDto::from(progress))
        }
        DomainEvent::ReadProgressDeleted(progress) => {
            event("ReadProgressDeleted", &ReadProgressSseDto::from(progress))
        }
        DomainEvent::ReadProgressSeriesChanged(progress) => event(
            "ReadProgressSeriesChanged",
            &ReadProgressSeriesSseDto::from(progress),
        ),
        DomainEvent::ReadProgressSeriesDeleted(progress) => event(
            "ReadProgressSeriesDeleted",
            &ReadProgressSeriesSseDto::from(progress),
        ),
        DomainEvent::SessionsExpired(user_id) => {
            event("SessionExpired", &SessionExpiredSseDto { user_id })
        }
        DomainEvent::BookImported(import) => event("BookImported", &BookImportSseDto::from(import)),
        DomainEvent::ThumbnailBookAdded(thumbnail) => {
            event("ThumbnailBookAdded", &ThumbnailBookSseDto::from(thumbnail))
        }
        DomainEvent::ThumbnailBookDeleted(thumbnail) => event(
            "ThumbnailBookDeleted",
            &ThumbnailBookSseDto::from(thumbnail),
        ),
        DomainEvent::ThumbnailSeriesAdded(thumbnail) => event(
            "ThumbnailSeriesAdded",
            &ThumbnailSeriesSseDto::from(thumbnail),
        ),
        DomainEvent::ThumbnailSeriesDeleted(thumbnail) => event(
            "ThumbnailSeriesDeleted",
            &ThumbnailSeriesSseDto::from(thumbnail),
        ),
        DomainEvent::ThumbnailSetAdded(thumbnail) => set_thumbnail_event("Added", thumbnail),
        DomainEvent::ThumbnailSetDeleted(thumbnail) => set_thumbnail_event("Deleted", thumbnail),
    }
}

/// `ThumbnailReadList{change}` or `ThumbnailSeriesCollection{change}`.
fn set_thumbnail_event(change: &str, thumbnail: &ThumbnailRef<SetId>) -> Event {
    let selected = thumbnail.selected;
    match &thumbnail.owner {
        SetId::ReadList(read_list_id) => event(
            &format!("ThumbnailReadList{change}"),
            &ThumbnailReadListSseDto {
                read_list_id,
                selected,
            },
        ),
        SetId::Collection(collection_id) => event(
            &format!("ThumbnailSeriesCollection{change}"),
            &ThumbnailSeriesCollectionSseDto {
                collection_id,
                selected,
            },
        ),
    }
}

fn event(name: &str, data: &impl Serialize) -> Event {
    Event::default()
        .event(name)
        .json_data(data)
        .expect("Event payloads serialize to JSON")
}

#[cfg(test)]
mod tests {
    use super::SseConnections;

    #[test]
    fn streams_beyond_the_limit_or_after_closing_are_refused() {
        let connections = SseConnections::with_limit(1);
        let slot = connections.open().expect("A free slot");
        assert!(connections.open().is_none());
        drop(slot);
        assert!(connections.open().is_some());
        connections.close();
        assert!(connections.open().is_none());
    }
}
