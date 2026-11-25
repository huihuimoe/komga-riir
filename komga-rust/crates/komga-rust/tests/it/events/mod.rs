//! `GET /sse/v1/events` over a real long-lived connection.

use std::time::Duration;

use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde_json::{Value, json};
use tokio::time::timeout;

mod content;
mod progress;
mod sessions;
mod sets;
mod thumbnails;

use crate::libraries;
use crate::read_lists::{PASSWORD, create_user};
use crate::support::TestServer;

pub(crate) const EVENTS: &str = "/sse/v1/events";

/// An open event stream, read one event at a time.
pub(crate) struct EventStream {
    response: Response,
    buffer: String,
}

impl EventStream {
    pub(crate) async fn open(request: RequestBuilder) -> Self {
        let response = request.send().await.expect("Event stream");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        Self {
            response,
            buffer: String::new(),
        }
    }

    /// The next event's name and data, skipping comments; `None` once the server ends the stream.
    pub(crate) async fn next(&mut self) -> Option<(String, Value)> {
        loop {
            if let Some(end) = self.buffer.find("\n\n") {
                let frame: String = self.buffer.drain(..end + 2).collect();
                if let Some(event) = parse(&frame) {
                    return Some(event);
                }
                continue;
            }
            let chunk = timeout(Duration::from_secs(10), self.response.chunk())
                .await
                .expect("An event within 10 seconds")
                .expect("Event stream body")?;
            self.buffer
                .push_str(std::str::from_utf8(&chunk).expect("UTF-8 events"));
        }
    }

    /// The next event other than `TaskQueueStatus`.
    async fn next_change(&mut self) -> (String, Value) {
        loop {
            let event = self.next().await.expect("An open stream");
            if event.0 != "TaskQueueStatus" {
                return event;
            }
        }
    }
}

fn parse(frame: &str) -> Option<(String, Value)> {
    let mut name = None;
    let mut data = None;
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            name = Some(value.trim_start().to_owned());
        } else if let Some(value) = line.strip_prefix("data:") {
            data = Some(serde_json::from_str(value.trim_start()).expect("JSON data"));
        }
    }
    Some((name?, data.expect("Data of the event")))
}

async fn library_request(server: &TestServer, method: Method, path: &str, body: Option<Value>) {
    let mut request = server.admin(method.clone(), path);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.expect("Library request");
    assert_eq!(response.status(), StatusCode::NO_CONTENT, "{method} {path}");
}

#[tokio::test]
async fn library_changes_reach_those_who_may_see_the_library() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    for root in ["a", "b"] {
        std::fs::create_dir(directory.path().join(root)).expect("Library root");
    }
    let server = TestServer::start_claimed(directory.path()).await;
    let mut admin = EventStream::open(server.admin(Method::GET, EVENTS)).await;
    let (name, status) = admin.next().await.expect("Task queue status");
    assert_eq!(name, "TaskQueueStatus");
    assert!(
        status["count"].is_i64() && status["countByType"].is_object(),
        "{status}"
    );

    let a = libraries::create(&server, "A", &directory.path().join("a")).await;
    let b = libraries::create(&server, "B", &directory.path().join("b")).await;
    for library in [&a, &b] {
        let added = (
            "LibraryAdded".to_owned(),
            json!({"libraryId": library["id"]}),
        );
        assert_eq!(admin.next_change().await, added);
    }

    let access = json!({"sharedLibraries": {"all": false, "libraryIds": [b["id"]]}});
    create_user(&server, "reader@example.org", access).await;
    let reader = server
        .request(Method::GET, EVENTS)
        .basic_auth("reader@example.org", Some(PASSWORD));
    let mut reader = EventStream::open(reader).await;
    let rename = |name| Some(json!({"name": name}));
    library_request(&server, Method::PATCH, &libraries::route(&a), rename("A2")).await;
    library_request(&server, Method::PATCH, &libraries::route(&b), rename("B2")).await;
    library_request(&server, Method::DELETE, &libraries::route(&b), None).await;
    let changed = |library: &Value| {
        (
            "LibraryChanged".to_owned(),
            json!({"libraryId": library["id"]}),
        )
    };
    let deleted = ("LibraryDeleted".to_owned(), json!({"libraryId": b["id"]}));
    // The reader sees neither library A nor the task queue.
    assert_eq!(reader.next().await, Some(changed(&b)));
    assert_eq!(reader.next().await, Some(deleted.clone()));
    for expected in [changed(&a), changed(&b), deleted] {
        assert_eq!(admin.next_change().await, expected);
    }

    // Shutting down ends open streams instead of waiting for them.
    server.stop().await;
    while admin.next().await.is_some() {}
    assert_eq!(reader.next().await, None);
}
