//! Both OPDS versions behind a proxy and for restricted readers.

use reqwest::header::LINK;
use reqwest::{Method, RequestBuilder, StatusCode};
use serde_json::{Value, json};

use crate::books::library_with_books;
use crate::books::reading::access::create_reader;
use crate::books::reading::{fetch, pages_book};
use crate::support::db::content::book;
use crate::support::db::seed;

const PROXY: &str = "https://proxy.example/komga/";

fn proxied(request: RequestBuilder) -> RequestBuilder {
    request
        .header("X-Forwarded-Proto", "https")
        .header("X-Forwarded-Host", "proxy.example")
        .header("X-Forwarded-Prefix", "/komga/")
}

/// Every `href` or `template` attribute of an Atom or OpenSearch document.
fn xml_links(body: &[u8]) -> Vec<String> {
    let body = std::str::from_utf8(body).expect("UTF-8");
    ["href=\"", "template=\""]
        .iter()
        .flat_map(|attribute| body.split(attribute).skip(1))
        .map(|rest| rest[..rest.find('"').expect("Closing quote")].to_owned())
        .collect()
}

/// Every `href` of an OPDS 2 document.
fn json_links(value: &Value, links: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                match value {
                    Value::String(href) if key == "href" => links.push(href.clone()),
                    _ => json_links(value, links),
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| json_links(item, links)),
        _ => {}
    }
}

fn assert_proxied(path: &str, links: &[String]) {
    assert!(!links.is_empty(), "{path}");
    for link in links {
        let komga =
            link.starts_with(PROXY) || link.starts_with("https://github.com/huihuimoe/komga-riir");
        assert!(komga || link == "https://komga.org", "{path}: {link}");
    }
}

#[tokio::test]
async fn links_follow_the_proxy() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Alpha/one.cbz", pages_book())]).await;
    let (_, series) = book(&pool, "one").await;

    for path in [
        "/opds/v1.2/catalog".to_owned(),
        "/opds/v1.2/search".to_owned(),
        format!("/opds/v1.2/series/{series}"),
    ] {
        let fetched = fetch(proxied(server.admin(Method::GET, &path))).await;
        assert_eq!(fetched.status, StatusCode::OK, "{path}");
        assert_proxied(&path, &xml_links(&fetched.body));
    }
    for path in [
        "/opds/v2/catalog".to_owned(),
        format!("/opds/v2/series/{series}"),
        "/opds/v2/auth".to_owned(),
    ] {
        let fetched = fetch(proxied(server.admin(Method::GET, &path))).await;
        assert_eq!(fetched.status, StatusCode::OK, "{path}");
        let mut links = Vec::new();
        json_links(
            &serde_json::from_slice(&fetched.body).expect("JSON"),
            &mut links,
        );
        assert_proxied(&path, &links);
    }

    let anonymous = fetch(proxied(server.request(Method::GET, "/opds/v2/catalog"))).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert!(
        anonymous
            .header(&LINK)
            .starts_with(&format!("<{PROXY}opds/v2/auth>")),
        "{}",
        anonymous.header(&LINK)
    );
    server.stop().await;
}

#[tokio::test]
async fn restricted_readers_see_and_fetch_only_what_they_may() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = [
        ("Adult/one.cbz", pages_book()),
        ("Kids/two.cbz", pages_book()),
    ];
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let (adult_book, adult) = book(&pool, "one").await;
    let (kids_book, _) = book(&pool, "two").await;
    seed::series_age_rating(&pool, &adult, 18).await;
    let reader = create_reader(
        &server,
        "kid@example.com",
        json!({
            "roles": ["PAGE_STREAMING"],
            "sharedLibraries": {"all": true, "libraryIds": []},
            "ageRestriction": {"age": 16, "restriction": "EXCLUDE"},
        }),
    )
    .await;
    let get = |path: String| fetch(reader.get(&server, &path));

    let v1 = get("/opds/v1.2/series".to_owned()).await;
    let v1_links = xml_links(&v1.body);
    assert!(
        v1_links.iter().all(|link| !link.contains(&adult)),
        "{v1_links:?}"
    );
    let v2 = get("/opds/v2/libraries/browse".to_owned()).await;
    let mut v2_links = Vec::new();
    json_links(
        &serde_json::from_slice(&v2.body).expect("JSON"),
        &mut v2_links,
    );
    assert!(
        v2_links.iter().all(|link| !link.contains(&adult)),
        "{v2_links:?}"
    );
    assert!(
        v2_links.iter().any(|link| link.contains("/series/")),
        "{v2_links:?}"
    );

    for path in [
        format!("/opds/v1.2/series/{adult}"),
        format!("/opds/v2/series/{adult}"),
        format!("/opds/v1.2/books/{adult_book}/pages/0"),
        format!("/opds/v2/books/{adult_book}/pages/1"),
        // Streaming pages does not let the reader download the book.
        format!("/opds/v1.2/books/{kids_book}/file/two.cbz"),
        format!("/opds/v2/books/{kids_book}/file"),
    ] {
        assert_eq!(
            get(path.clone()).await.status,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    let page = get(format!("/opds/v2/books/{kids_book}/pages/1")).await;
    assert_eq!(page.status, StatusCode::OK);
    server.stop().await;
}
