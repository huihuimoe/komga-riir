//! Kobo library sync and reading states (`KoboController.syncLibrary`, `getState`,
//! `updateState`).

use std::fs;
use std::path::Path;

use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

use super::api_key;
use crate::books::epub::sample;
use crate::books::library_with_books;
use crate::books::read_progress::mark;
use crate::support::db::content::book;
use crate::support::tasks::wait_for_tasks;
use crate::support::{TestServer, json};

const NOVEL: &str = "The Incomplete Theft - Ralph Burke.epub";
/// A resource of the novel's reading order.
const CHAPTER: &str =
    "OEBPS/@public@vhost@g@gutenberg@html@files@65659@65659-h@65659-h-0.htm_split_002.html";

/// A library of `names` novels synced at most `limit` items a page.
async fn library(directory: &Path, names: &[&str], limit: u32) -> (TestServer, Vec<String>) {
    fs::write(
        directory.join("application.yml"),
        format!("komga:\n  kobo:\n    sync-item-limit: {limit}\n"),
    )
    .expect("Configuration");
    let books: Vec<_> = names
        .iter()
        .map(|name| (format!("Series/{name}.epub"), sample(NOVEL)))
        .collect();
    let books: Vec<_> = books
        .iter()
        .map(|(path, epub)| (path.as_str(), epub.clone()))
        .collect();
    let (server, _, pool) = library_with_books(directory, &books).await;
    let mut ids = Vec::new();
    for name in names {
        ids.push(book(&pool, name).await.0);
    }
    (server, ids)
}

struct Page {
    items: Vec<Value>,
    token: String,
    more: bool,
}

async fn sync(server: &TestServer, key: &str, token: Option<&str>) -> Page {
    let mut request = server.request(Method::GET, &format!("/kobo/{key}/v1/library/sync"));
    if let Some(token) = token {
        request = request.header("X-Kobo-SyncToken", token);
    }
    let response = request.send().await.expect("Library sync");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    let more = headers
        .get("x-kobo-sync")
        .is_some_and(|value| value == "continue");
    let token = headers["x-kobo-synctoken"]
        .to_str()
        .expect("Sync token")
        .to_owned();
    assert!(token.starts_with("KOMGA."), "{token}");
    let items = json(response).await.as_array().expect("Sync items").clone();
    Page { items, token, more }
}

/// `(kind, book or read list id)` of each item.
fn kinds(page: &Page) -> Vec<(String, String)> {
    page.items
        .iter()
        .map(|item| {
            let (kind, body) = item
                .as_object()
                .expect("Sync item")
                .iter()
                .next()
                .expect("Kind");
            let id = body
                .pointer("/BookEntitlement/Id")
                .or_else(|| body.pointer("/ReadingState/EntitlementId"))
                .or_else(|| body.pointer("/Tag/Id"))
                .or_else(|| body.get("EntitlementId"))
                .and_then(Value::as_str)
                .expect("Item id");
            (kind.clone(), id.to_owned())
        })
        .collect()
}

#[tokio::test]
async fn a_device_syncs_page_by_page_then_only_what_changed() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, ids) = library(directory.path(), &["a", "b", "c"], 2).await;
    let key = api_key(&server, None).await;

    let first = sync(&server, &key, None).await;
    assert!(first.more);
    let second = sync(&server, &key, Some(&first.token)).await;
    assert!(!second.more);
    let mut handed_out: Vec<_> = kinds(&first).into_iter().chain(kinds(&second)).collect();
    handed_out.sort();
    let mut expected: Vec<_> = ids
        .iter()
        .map(|id| ("NewEntitlement".to_owned(), id.clone()))
        .collect();
    expected.sort();
    assert_eq!(handed_out, expected);
    let entitlement = &first.items[0]["NewEntitlement"];
    assert_eq!(entitlement["BookEntitlement"]["IsRemoved"], false);
    assert_eq!(
        entitlement["BookMetadata"]["DownloadUrls"][0]["Format"],
        "KEPUB"
    );
    assert_eq!(
        entitlement["ReadingState"]["StatusInfo"]["Status"],
        "ReadyToRead"
    );

    // A retried page is not handed out again: the device already holds it.
    let retried = sync(&server, &key, Some(&first.token)).await;
    assert!(retried.items.is_empty() && !retried.more);
    let unchanged = sync(&server, &key, Some(&second.token)).await;
    assert!(unchanged.items.is_empty() && !unchanged.more);

    let response = mark(&server, &ids[1], json!({"completed": true})).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let changes = sync(&server, &key, Some(&unchanged.token)).await;
    assert_eq!(
        kinds(&changes),
        [
            ("ChangedReadingState".to_owned(), ids[1].clone()),
            // Kobo gets On Deck as a tag; user read lists are not synced.
            ("NewTag".to_owned(), "KOMGA-ONDECK".to_owned()),
        ]
    );
    let state = &changes.items[0]["ChangedReadingState"]["ReadingState"];
    assert_eq!(state["StatusInfo"]["Status"], "Finished");
    let tag = &changes.items[1]["NewTag"]["Tag"];
    assert_eq!(tag["Name"], "On Deck");
    assert_eq!(
        tag["Items"],
        json!([{"RevisionId": ids[0], "Type": "ProductRevisionTagItem"}])
    );

    // An unreadable token starts over from scratch.
    let restarted = sync(&server, &key, Some("garbage")).await;
    assert_eq!(restarted.items.len(), 2);
    assert!(restarted.more);
    server.stop().await;
}

async fn put_state(server: &TestServer, key: &str, book_id: &str, state: Value) -> Response {
    server
        .request(
            Method::PUT,
            &format!("/kobo/{key}/v1/library/{book_id}/state"),
        )
        .json(&json!({"ReadingStates": [state]}))
        .send()
        .await
        .expect("Reading state update")
}

async fn get_state(server: &TestServer, key: &str, book_id: &str) -> Value {
    let response = server
        .request(
            Method::GET,
            &format!("/kobo/{key}/v1/library/{book_id}/state"),
        )
        .send()
        .await
        .expect("Reading state");
    assert_eq!(response.status(), StatusCode::OK);
    json(response).await[0].take()
}

fn reading(modified: &str, status: &str, location: Value) -> Value {
    json!({
        "LastModified": modified,
        "CurrentBookmark": {
            "ProgressPercent": 40.0,
            "ContentSourceProgressPercent": 50.0,
            "Location": location,
        },
        "StatusInfo": {"Status": status},
    })
}

async fn result(response: Response) -> String {
    assert_eq!(response.status(), StatusCode::OK);
    json(response).await["RequestResult"]
        .as_str()
        .expect("Result")
        .to_owned()
}

#[tokio::test]
async fn a_device_reports_reading_states_of_its_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, ids) = library(directory.path(), &["a"], 100).await;
    let key = api_key(&server, None).await;
    let id = &ids[0];
    let location = json!({"Value": "kobo.1.1", "Type": "KoboSpan", "Source": CHAPTER});

    let unread = get_state(&server, &key, id).await;
    assert_eq!(unread["StatusInfo"]["Status"], "ReadyToRead");
    assert_eq!(unread["StatusInfo"]["TimesStartedReading"], 0);

    let update = reading("2026-01-02T00:00:00Z", "Reading", location.clone());
    assert_eq!(
        result(put_state(&server, &key, id, update).await).await,
        "Success"
    );
    let state = get_state(&server, &key, id).await;
    assert_eq!(state["StatusInfo"]["Status"], "Reading");
    let bookmark = &state["CurrentBookmark"];
    assert_eq!(bookmark["ContentSourceProgressPercent"], 50.0);
    assert_eq!(bookmark["Location"], location);

    // Kobo replays older states; they are refused, not recorded.
    let older = reading("2026-01-01T00:00:00Z", "Reading", location.clone());
    assert_eq!(
        result(put_state(&server, &key, id, older).await).await,
        "Failure"
    );
    let finished = reading("2026-01-03T00:00:00Z", "Finished", location.clone());
    assert_eq!(
        result(put_state(&server, &key, id, finished).await).await,
        "Success"
    );
    assert_eq!(
        get_state(&server, &key, id).await["StatusInfo"]["Status"],
        "Finished"
    );

    let without_location = reading("2026-01-04T00:00:00Z", "Reading", Value::Null);
    let response = put_state(&server, &key, id, without_location).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let unknown = put_state(
        &server,
        &key,
        "UNKNOWN",
        reading("2026-01-04T00:00:00Z", "Reading", location),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    server.stop().await;
}

#[tokio::test]
async fn edited_and_deleted_books_reach_the_device() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, ids) = library(directory.path(), &["a", "b"], 100).await;
    let key = api_key(&server, None).await;
    let synced = sync(&server, &key, None).await;
    assert_eq!(synced.items.len(), 2);

    let edit = server
        .admin(Method::PATCH, &format!("/api/v1/books/{}/metadata", ids[0]))
        .json(&json!({"title": "Edited"}))
        .send()
        .await
        .expect("Metadata update");
    assert_eq!(edit.status(), StatusCode::NO_CONTENT);
    let delete = server
        .admin(Method::DELETE, &format!("/api/v1/books/{}/file", ids[1]))
        .send()
        .await
        .expect("Book deletion");
    assert_eq!(delete.status(), StatusCode::ACCEPTED);
    wait_for_tasks(directory.path(), |tasks| tasks.is_empty()).await;

    let changes = sync(&server, &key, Some(&synced.token)).await;
    assert_eq!(
        kinds(&changes),
        [
            ("NewEntitlement".to_owned(), ids[0].clone()),
            ("ChangedProductMetadata".to_owned(), ids[0].clone()),
            ("ChangedEntitlement".to_owned(), ids[1].clone()),
        ]
    );
    assert_eq!(
        changes.items[1]["ChangedProductMetadata"]["Title"],
        "Edited"
    );
    let removed = &changes.items[2]["ChangedEntitlement"];
    assert_eq!(removed["BookEntitlement"]["IsRemoved"], true);
    assert_eq!(removed["BookMetadata"]["Title"], ids[1].as_str());
    assert_eq!(removed["BookMetadata"]["DownloadUrls"], json!([]));
    assert_eq!(removed["ReadingState"], Value::Null);
    server.stop().await;
}
