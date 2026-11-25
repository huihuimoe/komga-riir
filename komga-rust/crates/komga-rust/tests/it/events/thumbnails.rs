//! Thumbnail events, sent to users who see what the thumbnail belongs to.

use reqwest::Method;
use serde_json::{Value, json};

use super::{EVENTS, EventStream};
use crate::read_lists::{Fixture, PASSWORD, create_list, create_user, fixture, send};
use crate::support::json;
use crate::support::posters::{file, png};

/// The next thumbnail event, past events left from the scan.
async fn next_thumbnail(stream: &mut EventStream) -> (String, Value) {
    loop {
        let event = stream.next_change().await;
        if event.0.starts_with("Thumbnail") {
            return event;
        }
    }
}

/// Uploads a thumbnail to `owner` (`books/{id}`, …) and returns its ID.
async fn upload(f: &Fixture, owner: &str, selected: bool) -> String {
    let path = format!("/api/v1/{owner}/thumbnails?selected={selected}");
    let request = f
        .server
        .admin(Method::POST, &path)
        .multipart(file(png(8, 8)));
    let uploaded = json(request.send().await.expect("Upload")).await;
    uploaded["id"].as_str().expect("Thumbnail ID").to_owned()
}

fn event(name: &str, data: Value) -> (String, Value) {
    (name.to_owned(), data)
}

#[tokio::test]
async fn thumbnail_changes_reach_users_who_see_their_owner() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let access = json!({"sharedLibraries": {"all": false, "libraryIds": [f.library_a]}});
    create_user(&f.server, "reader@example.org", access).await;
    let mut admin = EventStream::open(f.server.admin(Method::GET, EVENTS)).await;
    let reader = f
        .server
        .request(Method::GET, EVENTS)
        .basic_auth("reader@example.org", Some(PASSWORD));
    let mut reader = EventStream::open(reader).await;
    let only_b = create_list(&f, "Only B", &[&f.b1]).await;
    let body = json!({"name": "A", "ordered": false, "seriesIds": [f.series_a]});
    let created = send(
        &f.server,
        None,
        Method::POST,
        "/api/v1/collections",
        Some(&body),
    )
    .await;
    let collection = json(created).await["id"].as_str().expect("ID").to_owned();

    upload(&f, &format!("books/{}", f.b1), true).await;
    let a1 = upload(&f, &format!("books/{}", f.a1), false).await;
    let path = format!("/api/v1/books/{}/thumbnails/{a1}", f.a1);
    send(&f.server, None, Method::DELETE, &path, None).await;
    upload(&f, &format!("series/{}", f.series_a), true).await;
    upload(&f, &format!("readlists/{only_b}"), true).await;
    upload(&f, &format!("collections/{collection}"), false).await;

    let b1_added = json!({"bookId": f.b1, "seriesId": f.series_b, "selected": true});
    let a1_thumbnail = json!({"bookId": f.a1, "seriesId": f.series_a, "selected": false});
    let seen_by_both = [
        event("ThumbnailBookAdded", a1_thumbnail.clone()),
        event("ThumbnailBookDeleted", a1_thumbnail),
        event(
            "ThumbnailSeriesAdded",
            json!({"seriesId": f.series_a, "selected": true}),
        ),
    ];
    let collection_added = event(
        "ThumbnailSeriesCollectionAdded",
        json!({"collectionId": collection, "selected": false}),
    );
    assert_eq!(
        next_thumbnail(&mut admin).await,
        event("ThumbnailBookAdded", b1_added)
    );
    for expected in &seen_by_both {
        assert_eq!(&next_thumbnail(&mut admin).await, expected);
        assert_eq!(&next_thumbnail(&mut reader).await, expected);
    }
    assert_eq!(
        next_thumbnail(&mut admin).await,
        event(
            "ThumbnailReadListAdded",
            json!({"readListId": only_b, "selected": true}),
        )
    );
    assert_eq!(next_thumbnail(&mut admin).await, collection_added);
    // The book and read list of library B are skipped.
    assert_eq!(next_thumbnail(&mut reader).await, collection_added);
}
