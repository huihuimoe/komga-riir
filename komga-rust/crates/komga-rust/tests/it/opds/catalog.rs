//! The OPDS 1.2 catalog, browsed the way a reader app does.

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde_json::json;

use crate::books::reading::access::create_reader;
use crate::books::reading::{Fetched, fetch};
use crate::books::{GIF, JPEG, library_with_books};
use crate::support::TestServer;
use crate::support::cbz::{self, STORED};

const ATOM: &str = "application/atom+xml";

/// The `link` elements of a feed or description, as their attributes, in document order.
fn links(fetched: &Fetched) -> Vec<Vec<(String, String)>> {
    let mut reader = Reader::from_reader(fetched.body.as_slice());
    let mut buffer = Vec::new();
    let mut links = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer).expect("XML") {
            Event::Empty(element) if matches!(element.name().as_ref(), "link" | "Url") => {
                links.push(attributes(&element));
            }
            Event::Eof => return links,
            _ => {}
        }
        buffer.clear();
    }
}

fn attributes(element: &BytesStart<'_>) -> Vec<(String, String)> {
    element
        .attributes()
        .map(|attribute| {
            let attribute = attribute.expect("Attribute");
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .expect("Attribute value");
            let key = attribute.key.as_ref().to_owned();
            (key, value.into_owned())
        })
        .collect()
}

fn attribute<'a>(link: &'a [(String, String)], name: &str) -> Option<&'a str> {
    link.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// The `href` of the first link with `rel`, relative to the server.
fn href(fetched: &Fetched, rel: &str) -> Option<String> {
    links(fetched)
        .iter()
        .find(|link| attribute(link, "rel") == Some(rel))
        .map(|link| relative(attribute(link, "href").expect("href")))
}

fn relative(href: &str) -> String {
    let start = href.find("/opds/").expect("An OPDS link");
    href[start..].to_owned()
}

async fn get(server: &TestServer, path: &str) -> Fetched {
    let fetched = fetch(server.admin(Method::GET, path)).await;
    assert_eq!(
        fetched.status,
        StatusCode::OK,
        "{path}: {}",
        fetched.message()
    );
    fetched
}

fn pages() -> Vec<u8> {
    cbz::zip(&[
        ("1.jpg", STORED, &cbz::fixture(JPEG)),
        ("2.gif", STORED, &cbz::fixture(GIF)),
    ])
}

#[tokio::test]
async fn a_book_is_reached_from_the_catalog_through_search() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let book = pages();
    let books = [("Alpha/one.cbz", book.clone()), ("Beta/two.cbz", pages())];
    let (server, _, _) = library_with_books(directory.path(), &books).await;

    let catalog = get(&server, "/opds/v1.2/catalog").await;
    assert_eq!(catalog.header(&CONTENT_TYPE), ATOM);
    let description = get(&server, &href(&catalog, "search").expect("search")).await;
    let template = links(&description)
        .iter()
        .find_map(|link| attribute(link, "template").map(relative))
        .expect("Search template");
    assert_eq!(template, "/opds/v1.2/series?search={searchTerms}");

    let found = get(&server, &template.replace("{searchTerms}", "alp")).await;
    let series = href(&found, "subsection").expect("Matching series");
    assert_eq!(
        links(&found)
            .iter()
            .filter(|link| attribute(link, "rel") == Some("subsection"))
            .count(),
        1
    );
    let books = get(&server, &series).await;
    let acquisition = href(&books, "http://opds-spec.org/acquisition").expect("Acquisition");
    assert!(acquisition.ends_with("/file/one.cbz"), "{acquisition}");
    let file = get(&server, &acquisition).await;
    assert!(file.body == book, "the book file");

    // Mixed page types stream converted to JPEG, numbered from 0.
    let stream = links(&books)
        .into_iter()
        .find(|link| attribute(link, "rel") == Some("http://vaemendis.net/opds-pse/stream"))
        .expect("Page streaming");
    assert_eq!(attribute(&stream, "type"), Some("image/jpeg"));
    assert_eq!(attribute(&stream, "pse:count"), Some("2"));
    let page = relative(attribute(&stream, "href").expect("href")).replace("{pageNumber}", "1");
    assert!(page.ends_with("/pages/1?convert=jpeg"), "{page}");
    assert_eq!(
        get(&server, &page).await.header(&CONTENT_TYPE),
        "image/jpeg"
    );
    server.stop().await;
}

#[tokio::test]
async fn feeds_page_negotiate_and_check_access() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = [("Alpha/one.cbz", pages()), ("Beta/two.cbz", pages())];
    let (server, library, _) = library_with_books(directory.path(), &books).await;

    let first = get(&server, "/opds/v1.2/series?size=1").await;
    assert_eq!(href(&first, "previous"), None);
    let next = href(&first, "next").expect("next");
    assert_eq!(next, "/opds/v1.2/series?page=1");
    let second = get(&server, &next).await;
    assert_eq!(
        href(&second, "previous"),
        Some("/opds/v1.2/series?page=0".to_owned())
    );
    assert_eq!(href(&second, "next"), None);

    let xml = fetch(
        server
            .admin(Method::GET, "/opds/v1.2/catalog")
            .header(ACCEPT, "application/xml"),
    )
    .await;
    assert_eq!(xml.header(&CONTENT_TYPE), "application/xml");

    let library = format!(
        "/opds/v1.2/libraries/{}",
        library["id"].as_str().expect("id")
    );
    let reader = create_reader(
        &server,
        "hidden@example.com",
        json!({"sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    let denied = fetch(reader.get(&server, &library)).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    let missing = fetch(server.admin(Method::GET, "/opds/v1.2/series/missing")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let anonymous = fetch(server.request(Method::GET, "/opds/v1.2/catalog")).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

    for path in [
        "/opds/v1.2/ondeck",
        "/opds/v1.2/keep-reading",
        "/opds/v1.2/series/latest",
        "/opds/v1.2/books/latest",
        "/opds/v1.2/libraries",
        "/opds/v1.2/collections",
        "/opds/v1.2/readlists",
        "/opds/v1.2/publishers",
        &library,
    ] {
        get(&server, path).await;
    }
    server.stop().await;
}
