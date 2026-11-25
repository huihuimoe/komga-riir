//! Read list and collection events, listing only the members each user sees.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{EVENTS, EventStream};
use crate::read_lists::{PASSWORD, create_list, create_user, fixture, send};

fn list(name: &str, id: &str, books: &[&str]) -> (String, Value) {
    (name.to_owned(), json!({"readListId": id, "bookIds": books}))
}

fn collection(name: &str, id: &str, series: &[&str]) -> (String, Value) {
    (
        name.to_owned(),
        json!({"collectionId": id, "seriesIds": series}),
    )
}

/// The next read list or collection event, past content events left from the scan.
async fn next_set(stream: &mut EventStream) -> (String, Value) {
    loop {
        let event = stream.next_change().await;
        if event.0.starts_with("ReadList") || event.0.starts_with("Collection") {
            return event;
        }
    }
}

async fn expect_status(response: reqwest::Response, status: StatusCode) -> Value {
    assert_eq!(response.status(), status);
    if status == StatusCode::OK {
        response.json().await.expect("JSON body")
    } else {
        Value::Null
    }
}

#[tokio::test]
async fn set_changes_list_the_members_each_user_sees() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let server = &f.server;
    let mut admin = EventStream::open(server.admin(Method::GET, EVENTS)).await;
    let access = json!({"sharedLibraries": {"all": false, "libraryIds": [f.library_a]}});
    create_user(server, "reader@example.org", access).await;
    let reader = server
        .request(Method::GET, EVENTS)
        .basic_auth("reader@example.org", Some(PASSWORD));
    let mut reader = EventStream::open(reader).await;

    let mixed = create_list(&f, "Mixed", &[&f.a1, &f.b1]).await;
    let only_b = create_list(&f, "Only B", &[&f.b1]).await;
    let path = format!("/api/v1/readlists/{mixed}");
    let renamed = send(
        server,
        None,
        Method::PATCH,
        &path,
        Some(&json!({"name": "M"})),
    )
    .await;
    expect_status(renamed, StatusCode::NO_CONTENT).await;
    let path = format!("/api/v1/readlists/{only_b}");
    expect_status(
        send(server, None, Method::DELETE, &path, None).await,
        StatusCode::NO_CONTENT,
    )
    .await;

    let body = json!({"name": "C", "ordered": false, "seriesIds": [f.series_a, f.series_b]});
    let created = server
        .admin(Method::POST, "/api/v1/collections")
        .json(&body)
        .send()
        .await
        .expect("Collection creation");
    let created = expect_status(created, StatusCode::OK).await;
    let c = created["id"].as_str().expect("Collection ID").to_owned();
    let deleted = server
        .admin(Method::DELETE, &format!("/api/v1/collections/{c}"))
        .send()
        .await
        .expect("Collection deletion");
    expect_status(deleted, StatusCode::NO_CONTENT).await;

    let (a1, b1) = (f.a1.as_str(), f.b1.as_str());
    let (sa, sb) = (f.series_a.as_str(), f.series_b.as_str());
    for event in [
        list("ReadListAdded", &mixed, &[a1, b1]),
        list("ReadListAdded", &only_b, &[b1]),
        list("ReadListChanged", &mixed, &[a1, b1]),
        list("ReadListDeleted", &only_b, &[b1]),
        collection("CollectionAdded", &c, &[sa, sb]),
        collection("CollectionDeleted", &c, &[sa, sb]),
    ] {
        assert_eq!(next_set(&mut admin).await, event);
    }
    // The reader never saw "Only B" until it was gone, and sees no member of library B.
    for event in [
        list("ReadListAdded", &mixed, &[a1]),
        list("ReadListChanged", &mixed, &[a1]),
        list("ReadListDeleted", &only_b, &[]),
        collection("CollectionAdded", &c, &[sa]),
        collection("CollectionDeleted", &c, &[sa]),
    ] {
        assert_eq!(next_set(&mut reader).await, event);
    }
    f.server.stop().await;
}
