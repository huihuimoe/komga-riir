//! Read progress events, sent only to the user whose progress changed.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{EVENTS, EventStream};
use crate::read_lists::{Fixture, PASSWORD, create_user, fixture, send};

/// The next read progress event, past content events left from the scan.
async fn next_progress(stream: &mut EventStream) -> (String, Value) {
    loop {
        let event = stream.next_change().await;
        if event.0.starts_with("ReadProgress") {
            return event;
        }
    }
}

fn book(name: &str, book_id: &str, user_id: &str) -> (String, Value) {
    (
        name.to_owned(),
        json!({"bookId": book_id, "userId": user_id}),
    )
}

fn series(name: &str, series_id: &str, user_id: &str) -> (String, Value) {
    (
        name.to_owned(),
        json!({"seriesId": series_id, "userId": user_id}),
    )
}

async fn request(f: &Fixture, email: Option<&str>, method: Method, path: &str) {
    let completed = json!({"completed": true});
    let body = (method == Method::PATCH).then_some(&completed);
    let response = send(&f.server, email, method, path, body).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

pub(super) async fn user_id(f: &Fixture, email: Option<&str>) -> String {
    let response = send(&f.server, email, Method::GET, "/api/v2/users/me", None).await;
    let me: Value = response.json().await.expect("Current user");
    me["id"].as_str().expect("User ID").to_owned()
}

#[tokio::test]
async fn progress_changes_reach_only_their_user() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let reader_email = Some("reader@example.org");
    create_user(
        &f.server,
        "reader@example.org",
        json!({"sharedLibraries": {"all": true, "libraryIds": []}}),
    )
    .await;
    let admin_id = user_id(&f, None).await;
    let reader_id = user_id(&f, reader_email).await;
    let mut admin = EventStream::open(f.server.admin(Method::GET, EVENTS)).await;
    let reader = f
        .server
        .request(Method::GET, EVENTS)
        .basic_auth("reader@example.org", Some(PASSWORD));
    let mut reader = EventStream::open(reader).await;
    let b1 = format!("/api/v1/books/{}/read-progress", f.b1);
    let a1 = format!("/api/v1/books/{}/read-progress", f.a1);
    let series_a = format!("/api/v1/series/{}/read-progress", f.series_a);

    request(&f, reader_email, Method::PATCH, &b1).await;
    request(&f, None, Method::PATCH, &a1).await;
    request(&f, None, Method::DELETE, &a1).await;
    // Nothing to delete: no event.
    request(&f, None, Method::DELETE, &a1).await;
    request(&f, None, Method::PATCH, &a1).await;
    // a1 is already completed, so only a2 changes.
    request(&f, None, Method::POST, &series_a).await;
    request(&f, None, Method::DELETE, &series_a).await;
    request(&f, reader_email, Method::DELETE, &b1).await;

    assert_eq!(
        next_progress(&mut reader).await,
        book("ReadProgressChanged", &f.b1, &reader_id)
    );
    assert_eq!(
        next_progress(&mut reader).await,
        book("ReadProgressDeleted", &f.b1, &reader_id)
    );
    let changed = book("ReadProgressChanged", &f.a1, &admin_id);
    assert_eq!(next_progress(&mut admin).await, changed);
    assert_eq!(
        next_progress(&mut admin).await,
        book("ReadProgressDeleted", &f.a1, &admin_id)
    );
    assert_eq!(next_progress(&mut admin).await, changed);
    assert_eq!(
        next_progress(&mut admin).await,
        book("ReadProgressChanged", &f.a2, &admin_id)
    );
    assert_eq!(
        next_progress(&mut admin).await,
        series("ReadProgressSeriesChanged", &f.series_a, &admin_id)
    );
    // The series' books are deleted in no particular order.
    let mut deleted = [
        next_progress(&mut admin).await,
        next_progress(&mut admin).await,
    ];
    deleted.sort_by_key(|event| event.1.to_string());
    let mut expected = [
        book("ReadProgressDeleted", &f.a1, &admin_id),
        book("ReadProgressDeleted", &f.a2, &admin_id),
    ];
    expected.sort_by_key(|event| event.1.to_string());
    assert_eq!(deleted, expected);
    assert_eq!(
        next_progress(&mut admin).await,
        series("ReadProgressSeriesDeleted", &f.series_a, &admin_id)
    );
}
