//! Book posters by thumbnail ID: listing, reading, selecting and deleting.

use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::library_with_books;
use super::reading::access::create_reader;
use super::reading::{Fetched, fetch};
use super::thumbnails::{book_with_first_page, cover};
use crate::support::TestServer;
use crate::support::db::content::book;
use crate::support::posters::{list_at, status_and_body};
use crate::support::tasks::wait_for_generations;

pub(crate) fn route(book_id: &str) -> String {
    format!("/api/v1/books/{book_id}/thumbnails")
}

pub(crate) async fn list(server: &TestServer, book_id: &str) -> Vec<Value> {
    list_at(server, &route(book_id)).await
}

pub(crate) fn poster<'a>(posters: &'a [Value], kind: &str) -> &'a Value {
    posters
        .iter()
        .find(|poster| poster["type"] == kind)
        .unwrap_or_else(|| panic!("A {kind} poster in {posters:?}"))
}

/// Book `a` has a selected sidecar poster and a generated one; book `b` only a generated one.
pub(crate) async fn two_books(directory: &Path) -> (TestServer, String, String) {
    let (server, _, pool) = library_with_books(
        directory,
        &[
            ("A/a.cbz", book_with_first_page(&cover())),
            ("A/a.png", cover()),
            ("B/b.cbz", book_with_first_page(&cover())),
        ],
    )
    .await;
    wait_for_generations(directory).await;
    let (a, _) = book(&pool, "a").await;
    let (b, _) = book(&pool, "b").await;
    (server, a, b)
}

pub(crate) fn id_of(posters: &[Value], kind: &str) -> String {
    poster(posters, kind)["id"]
        .as_str()
        .expect("Poster ID")
        .to_owned()
}

#[tokio::test]
async fn posters_are_listed_read_and_selected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, a, _) = two_books(directory.path()).await;

    let posters = list(&server, &a).await;
    assert_eq!(posters.len(), 2, "{posters:?}");
    let sidecar = poster(&posters, "SIDECAR");
    let generated = poster(&posters, "GENERATED");
    assert_eq!(
        sidecar,
        &json!({
            "id": sidecar["id"], "bookId": a, "type": "SIDECAR", "selected": true,
            "mediaType": "image/png", "fileSize": cover().len(), "width": 800, "height": 400,
        })
    );
    assert_eq!(
        (
            &generated["selected"],
            &generated["width"],
            &generated["mediaType"]
        ),
        (&json!(false), &json!(300), &json!("image/jpeg"))
    );
    let sidecar_id = id_of(&posters, "SIDECAR");
    let generated_id = id_of(&posters, "GENERATED");

    let by_id: Fetched =
        fetch(server.admin(Method::GET, &format!("{}/{sidecar_id}", route(&a)))).await;
    assert_eq!(by_id.status, StatusCode::OK);
    assert!(by_id.body == cover(), "the sidecar bytes");

    // Selection moves to the requested poster only.
    let selected = format!("{}/{generated_id}/selected", route(&a));
    assert_eq!(
        status_and_body(server.admin(Method::PUT, &selected))
            .await
            .0,
        StatusCode::ACCEPTED
    );
    let posters = list(&server, &a).await;
    assert_eq!(poster(&posters, "GENERATED")["selected"], true);
    assert_eq!(poster(&posters, "SIDECAR")["selected"], false);
    server.stop().await;
}

#[tokio::test]
async fn invalid_poster_changes_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, a, b) = two_books(directory.path()).await;
    let posters = list(&server, &a).await;
    let sidecar_id = id_of(&posters, "SIDECAR");
    let generated_id = id_of(&posters, "GENERATED");

    // Only uploaded posters can be deleted; another book's poster is a bad request.
    let (status, body) =
        status_and_body(server.admin(Method::DELETE, &format!("{}/{sidecar_id}", route(&a)))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Only uploaded thumbnails can be deleted");
    for (method, path, status) in [
        (
            Method::PUT,
            format!("{}/{generated_id}/selected", route(&b)),
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::DELETE,
            format!("{}/{generated_id}", route(&b)),
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::PUT,
            format!("{}/missing/selected", route(&a)),
            StatusCode::NOT_FOUND,
        ),
        (
            Method::DELETE,
            format!("/api/v1/books/missing/thumbnails/{generated_id}"),
            StatusCode::NOT_FOUND,
        ),
        (
            Method::GET,
            format!("{}/missing", route(&a)),
            StatusCode::NOT_FOUND,
        ),
    ] {
        assert_eq!(
            status_and_body(server.admin(method, &path)).await.0,
            status,
            "{path}"
        );
    }
    assert_eq!(list(&server, &a).await.len(), 2, "nothing deleted");

    // As in Kotlin, unlimited users skip the book lookup: an unknown book lists nothing, and a
    // poster is served under any book ID.
    assert!(list(&server, "missing").await.is_empty());
    let elsewhere = fetch(server.admin(Method::GET, &format!("{}/{sidecar_id}", route(&b)))).await;
    assert_eq!(elsewhere.status, StatusCode::OK);
    server.stop().await;
}

#[tokio::test]
async fn posters_follow_content_access() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[("A/a.cbz", book_with_first_page(&cover()))],
    )
    .await;
    wait_for_generations(directory.path()).await;
    let (a, _) = book(&pool, "a").await;
    let posters = list(&server, &a).await;
    let id = posters[0]["id"].as_str().expect("ID");
    let poster_route = format!("{}/{id}", route(&a));

    create_reader(&server, "reader@example.com", json!({"roles": []})).await;
    create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    let as_user = |email: &str, method: Method, path: &str| {
        server
            .request(method, path)
            .basic_auth(email, Some("secret"))
    };
    for (request, status) in [
        (
            as_user("reader@example.com", Method::GET, &route(&a)),
            StatusCode::OK,
        ),
        (
            as_user("reader@example.com", Method::GET, &poster_route),
            StatusCode::OK,
        ),
        (
            as_user(
                "reader@example.com",
                Method::PUT,
                &format!("{poster_route}/selected"),
            ),
            StatusCode::FORBIDDEN,
        ),
        (
            as_user("reader@example.com", Method::DELETE, &poster_route),
            StatusCode::FORBIDDEN,
        ),
        (
            as_user("outsider@example.com", Method::GET, &route(&a)),
            StatusCode::FORBIDDEN,
        ),
        (
            as_user("outsider@example.com", Method::GET, &poster_route),
            StatusCode::FORBIDDEN,
        ),
        // A limited user looks the book up.
        (
            as_user("outsider@example.com", Method::GET, &route("missing")),
            StatusCode::NOT_FOUND,
        ),
        (
            server.request(Method::GET, &route(&a)),
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        assert_eq!(request.send().await.expect("Request").status(), status);
    }
    server.stop().await;
}
