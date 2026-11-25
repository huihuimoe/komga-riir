//! The book covers and pages OPDS feeds link to.

use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde_json::json;

use crate::books::book_posters::two_books;
use crate::books::reading::access::create_reader;
use crate::books::reading::{Fetched, fetch, pages_book};
use crate::books::thumbnails::assert_jpeg;
use crate::books::{GIF, library_with_books};
use crate::support::cbz;
use crate::support::db::content::book;

const V1: &str = "/opds/v1.2/books";
const V2: &str = "/opds/v2/books";

#[tokio::test]
async fn covers_are_served_as_jpeg() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, a, b) = two_books(directory.path()).await;
    let get = async |path: String| fetch(server.admin(Method::GET, &path)).await;

    // The selected sidecar is converted; the generated one is read again from the book.
    for id in [&a, &b] {
        for base in [V1, V2] {
            assert_jpeg(&get(format!("{base}/{id}/thumbnail")).await, (800, 400));
        }
    }
    // The small one is the selected thumbnail at the configured size.
    assert_jpeg(&get(format!("{V1}/{a}/thumbnail/small")).await, (300, 150));
    let generated = get(format!("/api/v1/books/{b}/thumbnail")).await;
    let small = get(format!("{V1}/{b}/thumbnail/small")).await;
    assert_jpeg(&small, (300, 150));
    assert!(small.body == generated.body, "the generated thumbnail");

    for path in [
        format!("{V1}/missing/thumbnail"),
        format!("{V2}/missing/thumbnail"),
        format!("{V1}/missing/thumbnail/small"),
    ] {
        assert_eq!(
            get(path.clone()).await.status,
            StatusCode::NOT_FOUND,
            "{path}"
        );
    }
    let hidden = create_reader(
        &server,
        "hidden@example.com",
        json!({"sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    let denied = fetch(hidden.get(&server, &format!("{V1}/{a}/thumbnail"))).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    server.stop().await;
}

#[tokio::test]
async fn pages_are_numbered_from_zero_and_need_page_streaming() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/book.cbz", pages_book())]).await;
    let (id, _) = book(&pool, "book").await;
    let pages = format!("{V1}/{id}/pages");
    let get = async |path: String| fetch(server.admin(Method::GET, &path)).await;

    let first = get(format!("{pages}/0")).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.header(&CONTENT_TYPE), "image/gif");
    assert!(first.body == cbz::fixture(GIF), "the stored page");
    let converted: Fetched = get(format!("{pages}/2?convert=jpeg")).await;
    assert_eq!(converted.status, StatusCode::OK);
    assert_eq!(converted.header(&CONTENT_TYPE), "image/jpeg");
    assert_eq!(
        get(format!("{pages}/one")).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(format!("{pages}/3")).await.status,
        StatusCode::BAD_REQUEST
    );

    let unauthenticated = fetch(server.request(Method::GET, &format!("{pages}/0"))).await;
    assert_eq!(unauthenticated.status, StatusCode::UNAUTHORIZED);
    let no_role = create_reader(&server, "no-role@example.com", json!({"roles": []})).await;
    let forbidden = fetch(no_role.get(&server, &format!("{pages}/0"))).await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
    server.stop().await;
}
