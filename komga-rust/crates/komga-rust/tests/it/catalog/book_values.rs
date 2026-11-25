//! Book-level referential values: authors and tags of `ReferentialV2Controller`, notable
//! entities of `ReferentialV3Controller`.

use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::books::library_with_books;
use crate::read_lists::{create_user, rate, send};
use crate::series::details::comic;
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

/// IDs of the catalog [`catalog`] builds.
pub(super) struct Catalog {
    pub(super) library: String,
    pub(super) alpha: String,
    pub(super) read_list: String,
    pub(super) collection: String,
}

/// `Alpha` (a1 by writer Ann and penciller Bób tagged `book tag` with character Rex and team
/// Squad, a2 by writer Zed) tagged `series tag`, `Beta` (b1 by writer Ann and colorist Cy tagged
/// `adult tag` with character Rex in location Paris) rated 18, and
/// `Gamma` without metadata; a read list and a collection hold `Beta` only.
pub(super) async fn catalog(directory: &Path) -> (TestServer, Catalog) {
    let a1 = "<ComicInfo><Writer>Ann</Writer><Penciller>Bób</Penciller>\
              <Tags>book tag</Tags><Characters>Rex</Characters><Teams>Squad</Teams></ComicInfo>";
    let b1 = "<ComicInfo><Writer>Ann</Writer><Colorist>Cy</Colorist>\
              <Tags>adult tag</Tags><Locations>Paris</Locations><Characters>Rex</Characters>\
              </ComicInfo>";
    let books = [
        ("Alpha/a1.cbz", comic(a1)),
        (
            "Alpha/a2.cbz",
            comic("<ComicInfo><Writer>Zed</Writer></ComicInfo>"),
        ),
        ("Beta/b1.cbz", comic(b1)),
        ("Gamma/g1.cbz", comic("<ComicInfo></ComicInfo>")),
    ];
    let (server, library, pool) = library_with_books(directory, &books).await;
    let alpha = book(&pool, "a1").await.1;
    let (b1, beta) = book(&pool, "b1").await;
    let path = format!("/api/v1/series/{alpha}/metadata");
    let tags = json!({"tags": ["series tag"]});
    let response = send(&server, None, Method::PATCH, &path, Some(&tags)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    rate(&server, &beta, 18).await;
    let read_list = json!({"name": "Beta books", "bookIds": [b1]});
    let read_list = create(&server, "/api/v1/readlists", &read_list).await;
    let collection = json!({"name": "Beta", "ordered": false, "seriesIds": [beta]});
    let collection = create(&server, "/api/v1/collections", &collection).await;
    let library = library["id"].as_str().expect("Library ID").to_owned();
    let catalog = Catalog {
        library,
        alpha,
        read_list,
        collection,
    };
    (server, catalog)
}

/// Creates the set `body` describes at `path` and returns its ID.
async fn create(server: &TestServer, path: &str, body: &Value) -> String {
    let response = send(server, None, Method::POST, path, Some(body)).await;
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    let created: Value = response.json().await.expect("Created set");
    created["id"].as_str().expect("Set ID").to_owned()
}

/// The page `email` (else the admin) gets from `path`.
async fn page(server: &TestServer, email: Option<&str>, path: &str) -> Value {
    let response = send(server, email, Method::GET, path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    openapi::checked(Method::GET, response).await
}

/// The values `email` (else the admin) gets from `path`, unpaged.
async fn values(server: &TestServer, email: Option<&str>, path: &str) -> Value {
    let separator = if path.contains('?') { '&' } else { '?' };
    let page = page(server, email, &format!("{path}{separator}unpaged=true")).await;
    let content = page["content"].as_array().expect("Content");
    assert_eq!(page["totalElements"], content.len(), "{path}");
    page["content"].clone()
}

pub(super) fn author(name: &str, role: &str) -> Value {
    json!({"name": name, "role": role})
}

#[tokio::test]
async fn authors_list_names_and_roles() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _) = catalog(directory.path()).await;
    let cases = [
        (
            "/api/v2/authors",
            json!([
                author("Ann", "writer"),
                author("Bób", "penciller"),
                author("Cy", "colorist"),
                author("Zed", "writer"),
            ]),
        ),
        (
            "/api/v2/authors?role=writer",
            json!([author("Ann", "writer"), author("Zed", "writer")]),
        ),
        (
            "/api/v2/authors?search=BOB",
            json!([author("Bób", "penciller")]),
        ),
        ("/api/v2/authors/names", json!(["Ann", "Bób", "Cy", "Zed"])),
        ("/api/v2/authors/names?role=writer&search=n", json!(["Ann"])),
        ("/api/v2/authors/names?role=nobody", json!([])),
        (
            "/api/v2/authors/roles",
            json!(["colorist", "penciller", "writer"]),
        ),
    ];
    for (path, expected) in cases {
        assert_eq!(values(&server, None, path).await, expected, "{path}");
    }

    let first = page(&server, None, "/api/v2/authors/names?size=3").await;
    assert_eq!(first["content"], json!(["Ann", "Bób", "Cy"]));
    assert_eq!(first["totalElements"], 4);
    let last = page(&server, None, "/api/v2/authors?size=3&page=1").await;
    assert_eq!(last["content"], json!([author("Zed", "writer")]));
    let anonymous = server.request(Method::GET, "/api/v2/authors").send().await;
    let anonymous = anonymous.expect("Request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn tags_cover_series_books_or_both() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, catalog) = catalog(directory.path()).await;
    let alpha = &catalog.alpha;
    let cases = [
        (
            "/api/v2/tags".to_owned(),
            json!(["adult tag", "book tag", "series tag"]),
        ),
        (
            "/api/v2/tags?include=SERIES".to_owned(),
            json!(["series tag"]),
        ),
        (
            "/api/v2/tags?include=BOOK".to_owned(),
            json!(["adult tag", "book tag"]),
        ),
        (
            "/api/v2/tags?include=BOTH&search=SERIES".to_owned(),
            json!(["series tag"]),
        ),
        (
            format!("/api/v2/tags?series_id={alpha}"),
            json!(["book tag", "series tag"]),
        ),
        (
            format!("/api/v2/tags?series_id={alpha}&include=BOOK"),
            json!(["book tag"]),
        ),
        (
            format!("/api/v2/tags?readlist_id={}", catalog.read_list),
            json!(["adult tag"]),
        ),
        (
            format!(
                "/api/v2/tags?readlist_id={}&include=SERIES",
                catalog.read_list
            ),
            json!([]),
        ),
    ];
    for (path, expected) in cases {
        assert_eq!(values(&server, None, &path).await, expected, "{path}");
    }
    let bad = send(
        &server,
        None,
        Method::GET,
        "/api/v2/tags?include=both",
        None,
    )
    .await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn filters_follow_their_precedence_within_the_user_scope() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, catalog) = catalog(directory.path()).await;
    let Catalog {
        library,
        alpha,
        read_list,
        collection,
    } = &catalog;
    let no_adults = json!({"ageRestriction": {"age": 18, "restriction": "EXCLUDE"}});
    create_user(&server, "minor@example.com", no_adults).await;
    let nothing = json!({"sharedLibraries": {"all": false, "libraryIds": []}});
    create_user(&server, "none@example.com", nothing).await;
    let minor = Some("minor@example.com");

    let names = "/api/v2/authors/names";
    let cases = [
        (
            None,
            format!("{names}?series_id={alpha}"),
            json!(["Ann", "Bób", "Zed"]),
        ),
        (
            None,
            format!("{names}?readlist_id={read_list}"),
            json!(["Ann", "Cy"]),
        ),
        (
            None,
            format!("{names}?collection_id={collection}"),
            json!(["Ann", "Cy"]),
        ),
        (
            None,
            format!("{names}?library_id={library}&series_id={alpha}"),
            json!(["Ann", "Bób", "Cy", "Zed"]),
        ),
        (
            None,
            format!("{names}?collection_id={collection}&series_id={alpha}"),
            json!(["Ann", "Cy"]),
        ),
        (
            None,
            format!("{names}?series_id={alpha}&readlist_id={read_list}"),
            json!(["Ann", "Bób", "Zed"]),
        ),
        (None, format!("{names}?library_id=other"), json!([])),
        (minor, names.to_owned(), json!(["Ann", "Bób", "Zed"])),
        (
            minor,
            "/api/v2/authors/roles".to_owned(),
            json!(["penciller", "writer"]),
        ),
        (
            minor,
            "/api/v2/tags".to_owned(),
            json!(["book tag", "series tag"]),
        ),
        (minor, format!("{names}?readlist_id={read_list}"), json!([])),
        (Some("none@example.com"), names.to_owned(), json!([])),
        (
            Some("none@example.com"),
            "/api/v2/tags".to_owned(),
            json!([]),
        ),
    ];
    for (email, path, expected) in cases {
        assert_eq!(
            values(&server, email, &path).await,
            expected,
            "{email:?} {path}"
        );
    }
}

fn entity(kind: &str, name: &str) -> Value {
    json!({"kind": kind, "name": name})
}

#[tokio::test]
async fn notable_entities_filter_like_authors() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, catalog) = catalog(directory.path()).await;
    create_user(
        &server,
        "minor@example.com",
        json!({"ageRestriction": {"age": 18, "restriction": "EXCLUDE"}}),
    )
    .await;
    let entities = "/api/v3/notable-entities";
    let alpha = &catalog.alpha;
    let read_list = &catalog.read_list;
    let cases = [
        (
            None,
            format!("{entities}?unpaged=true"),
            json!([
                entity("location", "Paris"),
                entity("character", "Rex"),
                entity("team", "Squad"),
            ]),
        ),
        (
            None,
            format!("{entities}?unpaged=true&kind=character&search=RE"),
            json!([entity("character", "Rex")]),
        ),
        (
            None,
            format!("{entities}/names?unpaged=true&series_id={alpha}"),
            json!(["Rex", "Squad"]),
        ),
        (
            None,
            format!("{entities}/names?unpaged=true&kind=team"),
            json!(["Squad"]),
        ),
        (
            None,
            format!("{entities}/kinds?unpaged=true"),
            json!(["character", "location", "team"]),
        ),
        (
            None,
            format!("{entities}/kinds?unpaged=true&readlist_id={read_list}"),
            json!(["character", "location"]),
        ),
        (
            Some("minor@example.com"),
            format!("{entities}/names?unpaged=true"),
            json!(["Rex", "Squad"]),
        ),
    ];
    for (email, path, expected) in cases {
        let page = page(&server, email, &path).await;
        assert_eq!(page["content"], expected, "{email:?} {path}");
    }
}

#[tokio::test]
async fn v3_pages_and_errors_have_their_own_shape() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _) = catalog(directory.path()).await;
    let entities = "/api/v3/notable-entities";
    let unpaged = page(&server, None, &format!("{entities}?unpaged=true&size=1")).await;
    assert_eq!(
        unpaged,
        json!({
            "content": unpaged["content"], "page": 0, "size": 3, "totalElements": 3,
            "totalPages": 1, "first": true, "last": true,
        })
    );
    let second = page(&server, None, &format!("{entities}/names?size=1&page=1")).await;
    assert_eq!(
        second,
        json!({
            "content": ["Rex"], "page": 1, "size": 1, "totalElements": 3,
            "totalPages": 3, "first": false, "last": false,
        })
    );
    let none = page(
        &server,
        None,
        &format!("{entities}/kinds?kind=x&library_id=none"),
    )
    .await;
    assert_eq!(
        none,
        json!({
            "content": [], "page": 0, "size": 20, "totalElements": 0,
            "totalPages": 0, "first": true, "last": true,
        })
    );

    let response = send(
        &server,
        None,
        Method::GET,
        &format!("{entities}?unpaged=maybe"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let content_type = response.headers()["content-type"].clone();
    assert_eq!(content_type, "application/problem+json");
    let problem: Value = response.json().await.expect("Problem detail");
    assert_eq!(
        problem,
        json!({
            "type": "about:blank", "title": "Bad Request", "status": 400,
            "detail": "Invalid value for parameter 'unpaged'", "instance": entities,
        })
    );
    let anonymous = server.request(Method::GET, entities).send().await;
    assert_eq!(
        anonymous.expect("Request").status(),
        StatusCode::UNAUTHORIZED
    );
}
