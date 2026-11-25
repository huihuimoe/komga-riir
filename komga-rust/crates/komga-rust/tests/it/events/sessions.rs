//! `SessionExpired`, sent only to the user whose sessions were revoked, and the streams that
//! revocation or logging out ends.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::progress::user_id;
use super::{EVENTS, EventStream};
use crate::read_lists::{Fixture, PASSWORD, create_user, fixture, send};

async fn open(f: &Fixture, email: &str) -> EventStream {
    let request = f
        .server
        .request(Method::GET, EVENTS)
        .basic_auth(email, Some(PASSWORD));
    EventStream::open(request).await
}

async fn admin(f: &Fixture, method: Method, path: &str, body: Option<&Value>) {
    let response = send(&f.server, None, method, path, body).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT, "{path}");
}

fn expired(user_id: &str) -> (String, Value) {
    ("SessionExpired".to_owned(), json!({"userId": user_id}))
}

#[tokio::test]
async fn revoked_sessions_are_announced_and_end_their_streams() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let all = json!({"sharedLibraries": {"all": true, "libraryIds": []}});
    for email in ["one@example.org", "two@example.org"] {
        create_user(&f.server, email, all.clone()).await;
    }
    let one = user_id(&f, Some("one@example.org")).await;
    let two = user_id(&f, Some("two@example.org")).await;
    let mut one_events = open(&f, "one@example.org").await;
    let mut two_events = open(&f, "two@example.org").await;
    let one_path = format!("/api/v2/users/{one}");
    let two_path = format!("/api/v2/users/{two}");

    admin(
        &f,
        Method::PATCH,
        &one_path,
        Some(&json!({"roles": ["FILE_DOWNLOAD"]})),
    )
    .await;
    admin(&f, Method::DELETE, &two_path, None).await;

    assert_eq!(one_events.next_change().await, expired(&one));
    assert_eq!(two_events.next_change().await, expired(&two));
    // The streams carried what the users could see before; they reconnect to get the rest.
    assert_eq!(one_events.next().await, None);
    assert_eq!(two_events.next().await, None);
}

#[tokio::test]
async fn logging_out_ends_the_stream_of_that_session() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let reader = "reader@example.org";
    create_user(
        &f.server,
        reader,
        json!({"sharedLibraries": {"all": true, "libraryIds": []}}),
    )
    .await;
    // A reader gets no `TaskQueueStatus`, so the stream has nothing to send before it ends.
    let response = f
        .server
        .request(Method::GET, "/api/v2/users/me")
        .basic_auth(reader, Some(PASSWORD))
        .header("X-Auth-Token", "")
        .send()
        .await
        .expect("Session");
    let token = response.headers()["x-auth-token"].clone();
    let by_session = f
        .server
        .request(Method::GET, EVENTS)
        .header("X-Auth-Token", token.clone());
    let mut logged_out = EventStream::open(by_session).await;
    let mut other = EventStream::open(f.server.admin(Method::GET, EVENTS)).await;

    let response = f
        .server
        .request(Method::POST, "/api/logout")
        .header("X-Auth-Token", token)
        .send()
        .await
        .expect("Logout");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(logged_out.next().await, None);
    let progress = format!("/api/v1/books/{}/read-progress", f.a1);
    let completed = json!({"completed": true});
    admin(&f, Method::PATCH, &progress, Some(&completed)).await;
    assert_eq!(other.next_change().await.0, "ReadProgressChanged");
}
