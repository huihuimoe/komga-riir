//! OPDS 2 feeds, from the catalog to a book file.

use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::books::library_with_books;
use crate::books::reading::access::{Reader, create_reader};
use crate::books::reading::{fetch, pages_book};
use crate::support::TestServer;
use crate::support::db::content::book;

async fn get(server: &TestServer, path: &str) -> Value {
    let fetched = fetch(server.admin(Method::GET, path)).await;
    assert_eq!(
        fetched.status,
        StatusCode::OK,
        "{path}: {}",
        fetched.message()
    );
    assert_eq!(fetched.header(&CONTENT_TYPE), "application/opds+json");
    serde_json::from_slice(&fetched.body).expect("JSON")
}

fn relative(href: &Value) -> String {
    let href = href.as_str().expect("href");
    let start = href.find("/opds/").expect("An OPDS link");
    href[start..].to_owned()
}

/// The `href` of the link with `rel` in `links`.
fn rel(links: &Value, rel: &str) -> Option<String> {
    links
        .as_array()
        .expect("links")
        .iter()
        .find(|link| link["rel"] == rel)
        .map(|link| relative(&link["href"]))
}

fn titles(links: &Value) -> Vec<&str> {
    links
        .as_array()
        .expect("links")
        .iter()
        .map(|link| {
            link["title"]
                .as_str()
                .unwrap_or_else(|| panic!("title in {link}"))
        })
        .collect()
}

fn group<'a>(feed: &'a Value, title: &str) -> &'a Value {
    feed["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .find(|group| group["metadata"]["title"] == title)
        .unwrap_or_else(|| panic!("group {title} in {feed}"))
}

#[tokio::test]
async fn a_book_is_reached_from_the_catalog_through_search() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = [
        ("Alpha/one.cbz", pages_book()),
        ("Beta/two.cbz", pages_book()),
    ];
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let (_, alpha) = book(&pool, "one").await;
    server
        .admin(Method::POST, "/api/v1/collections")
        .json(&json!({"name": "Picks", "ordered": false, "seriesIds": [alpha]}))
        .send()
        .await
        .expect("Collection");

    let catalog = get(&server, "/opds/v2/catalog").await;
    assert_eq!(catalog["metadata"]["title"], "All libraries - Recommended");
    assert_eq!(
        titles(&catalog["navigation"]),
        ["Recommended", "Browse", "Collections"]
    );
    let latest = group(&catalog, "Latest Books");
    assert_eq!(latest["publications"].as_array().expect("books").len(), 2);
    let search = catalog["links"]
        .as_array()
        .expect("links")
        .iter()
        .find(|link| link["rel"] == "search")
        .expect("search");
    assert_eq!(search["templated"], true);
    let template = relative(&search["href"]);
    assert_eq!(template, "/opds/v2/search{?query}");

    let found = get(&server, "/opds/v2/search?query=alp").await;
    assert_eq!(titles(&group(&found, "Series")["navigation"]), ["Alpha"]);
    let series = relative(&group(&found, "Series")["navigation"][0]["href"]);
    let books = get(&server, &series).await;
    assert_eq!(books["metadata"]["numberOfItems"], 1);
    let publication = &books["publications"][0];
    assert_eq!(publication["metadata"]["title"], "T");
    assert_eq!(
        publication["images"][0]["type"], "image/jpeg",
        "{publication}"
    );
    let file = rel(&publication["links"], "http://opds-spec.org/acquisition").expect("file");
    let downloaded = fetch(server.admin(Method::GET, &file)).await;
    assert!(downloaded.body == pages_book(), "the book file");

    let collections = get(&server, "/opds/v2/libraries/collections").await;
    let picks = relative(&group(&collections, "Collections")["navigation"][0]["href"]);
    let picked = get(&server, &picks).await;
    assert_eq!(titles(&picked["navigation"]), ["Alpha"]);
    server.stop().await;
}

#[tokio::test]
async fn feeds_page_and_check_access() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = [
        ("Alpha/one.cbz", pages_book()),
        ("Beta/two.cbz", pages_book()),
    ];
    let (server, library, _) = library_with_books(directory.path(), &books).await;
    let id = library["id"].as_str().expect("id");

    let browse = format!("/opds/v2/libraries/{id}/browse");
    let first = get(&server, &format!("{browse}?size=1")).await;
    assert_eq!(first["metadata"]["itemsPerPage"], 1);
    assert_eq!(first["metadata"]["currentPage"], 1);
    assert_eq!(first["metadata"]["numberOfItems"], 2);
    assert_eq!(rel(&first["links"], "previous"), None);
    let next = rel(&first["links"], "next").expect("next");
    assert_eq!(next, format!("{browse}?page=1"));
    let second = get(&server, &format!("{next}&size=1")).await;
    assert_eq!(titles(&group(&second, "Series")["navigation"]), ["Beta"]);
    assert_eq!(rel(&second["links"], "next"), None);

    let reader = create_reader(
        &server,
        "hidden@example.com",
        json!({"sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    let denied = fetch(reader.get(&server, &format!("/opds/v2/libraries/{id}"))).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let hidden = get_as(&reader, &server, "/opds/v2/libraries/books/latest").await;
    assert_eq!(hidden["metadata"]["numberOfItems"], 0);
    for missing in ["/opds/v2/series/missing", "/opds/v2/libraries/missing"] {
        let fetched = fetch(server.admin(Method::GET, missing)).await;
        assert_eq!(fetched.status, StatusCode::NOT_FOUND, "{missing}");
    }

    for path in [
        format!("/opds/v2/libraries/{id}"),
        "/opds/v2/libraries".to_owned(),
        "/opds/v2/libraries/keep-reading".to_owned(),
        format!("/opds/v2/libraries/{id}/on-deck"),
        format!("/opds/v2/libraries/{id}/series/latest"),
        "/opds/v2/libraries/readlists".to_owned(),
    ] {
        get(&server, &path).await;
    }
    server.stop().await;
}

async fn get_as(reader: &Reader, server: &TestServer, path: &str) -> Value {
    let fetched = fetch(reader.get(server, path)).await;
    assert_eq!(fetched.status, StatusCode::OK, "{path}");
    serde_json::from_slice(&fetched.body).expect("JSON")
}
