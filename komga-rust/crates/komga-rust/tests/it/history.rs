//! Historical events listed to admins (`HistoricalEventController`).

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::books::reading::access::create_reader;
use crate::books::reading::fetch;
use crate::file_deletion::{delete, library};
use crate::support::TestServer;
use crate::support::db::content::book;

async fn history(server: &TestServer, query: &str) -> Value {
    let response = fetch(server.admin(Method::GET, &format!("/api/v1/history{query}"))).await;
    assert_eq!(response.status, StatusCode::OK);
    serde_json::from_slice(&response.body).expect("JSON page")
}

/// `(type, name)` of each listed event.
fn events(page: &Value) -> Vec<(&str, &str)> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|event| {
            let name = event["properties"]["name"].as_str().expect("Name property");
            (event["type"].as_str().expect("Type"), name)
        })
        .collect()
}

#[tokio::test]
async fn admins_page_through_the_history_newest_first() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = library(directory.path()).await;
    let root = directory.path().join("comics");
    let (a, a_series) = book(&pool, "a").await;
    let (d, _) = book(&pool, "d").await;
    delete(&server, directory.path(), &a).await;
    delete(&server, directory.path(), &d).await;

    let newest = history(&server, "").await;
    let a_name = root.join("A").join("a.cbz").to_string_lossy().into_owned();
    let d_name = root.join("d.cbz").to_string_lossy().into_owned();
    assert_eq!(
        events(&newest),
        [
            ("BookFileDeleted", d_name.as_str()),
            ("BookFileDeleted", a_name.as_str())
        ]
    );
    let event = &newest["content"][1];
    assert_eq!(event["bookId"], a.as_str());
    assert_eq!(event["seriesId"], a_series.as_str());
    assert_eq!(
        event["properties"]["reason"],
        "File was deleted by user request"
    );
    assert!(
        event["timestamp"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z')),
        "{event}"
    );

    let oldest = history(&server, "?sort=timestamp,asc&size=1&page=1").await;
    assert_eq!(
        (&oldest["totalElements"], &oldest["totalPages"]),
        (&json!(2), &json!(2))
    );
    assert_eq!(events(&oldest), [("BookFileDeleted", d_name.as_str())]);
    // `Pageable` has no `unpaged` parameter here.
    let first = history(&server, "?unpaged=true&size=1").await;
    assert_eq!(events(&first), [("BookFileDeleted", d_name.as_str())]);

    let reader = create_reader(&server, "reader@example.com", json!({})).await;
    let denied = fetch(reader.request(&server, Method::GET, "/api/v1/history")).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    server.stop().await;
}
