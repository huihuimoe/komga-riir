//! Series-level referential values of `ReferentialV2Controller`: genres, sharing labels,
//! languages, publishers and release years.

use std::fs;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::books::library_with_books;
use crate::libraries::create;
use crate::read_lists::{create_user, send};
use crate::series::details::comic;
use crate::support::db::content::book;
use crate::support::tasks::wait_for_scan_work;
use crate::support::{TestServer, openapi};

/// Library `Comics` with `Alpha` (French drama and comedy from 1999), `Beta` (English horror for
/// adults from 2020) and `Gamma` (no metadata), and library `Other` with `Delta` (science
/// fiction from 2005). `Alpha` is labelled `kids` and `Beta` `adult`. Returns the server, the
/// `Comics` library ID and the ID of a collection holding `Alpha`.
pub(super) async fn catalog(directory: &Path) -> (TestServer, String, String) {
    let alpha = "<ComicInfo><Genre>Drama, Comédie</Genre><Publisher>Pub One</Publisher>\
                 <LanguageISO>fr</LanguageISO><Year>1999</Year></ComicInfo>";
    let beta = "<ComicInfo><Genre>Horror</Genre><Publisher>Ácme</Publisher>\
                <LanguageISO>en</LanguageISO><AgeRating>Adults Only 18+</AgeRating>\
                <Year>2020</Year></ComicInfo>";
    let books = [
        ("Alpha/a1.cbz", comic(alpha)),
        ("Beta/b1.cbz", comic(beta)),
        ("Gamma/g1.cbz", comic("<ComicInfo></ComicInfo>")),
    ];
    let (server, library, pool) = library_with_books(directory, &books).await;
    let other = directory.join("other");
    let delta = "<ComicInfo><Genre>Sci-Fi</Genre><Year>2005</Year></ComicInfo>";
    fs::create_dir_all(other.join("Delta")).expect("Series directory");
    fs::write(other.join("Delta/d1.cbz"), comic(delta)).expect("Book file");
    create(&server, "Other", &other).await;
    wait_for_scan_work(directory).await;

    let alpha = series_id(&pool, "a1").await;
    label(&server, &alpha, "kids").await;
    label(&server, &series_id(&pool, "b1").await, "adult").await;
    let body = json!({"name": "Kept", "ordered": false, "seriesIds": [alpha]});
    let response = send(
        &server,
        None,
        Method::POST,
        "/api/v1/collections",
        Some(&body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let collection: Value = response.json().await.expect("Collection");
    let library_id = library["id"].as_str().expect("Library ID").to_owned();
    let collection_id = collection["id"].as_str().expect("Collection ID").to_owned();
    (server, library_id, collection_id)
}

async fn series_id(pool: &SqlitePool, book_name: &str) -> String {
    book(pool, book_name).await.1
}

async fn label(server: &TestServer, series_id: &str, label: &str) {
    let path = format!("/api/v1/series/{series_id}/metadata");
    let body = json!({"sharingLabels": [label]});
    let response = send(server, None, Method::PATCH, &path, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
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
    assert_eq!(
        page["totalElements"],
        page["content"].as_array().expect("Content").len(),
        "{path}"
    );
    page["content"].clone()
}

#[tokio::test]
async fn values_are_distinct_non_blank_and_ordered() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, _) = catalog(directory.path()).await;
    let cases = [
        (
            "/api/v2/genres",
            json!(["comédie", "drama", "horror", "sci-fi"]),
        ),
        ("/api/v2/sharing-labels", json!(["adult", "kids"])),
        ("/api/v2/languages", json!(["en", "fr"])),
        ("/api/v2/publishers", json!(["Ácme", "Pub One"])),
        ("/api/v2/age-ratings", json!([18])),
        (
            "/api/v2/series/release-years",
            json!(["2020", "2005", "1999"]),
        ),
    ];
    for (path, expected) in cases {
        assert_eq!(values(&server, None, path).await, expected, "{path}");
    }

    let first = page(&server, None, "/api/v2/genres?size=3").await;
    assert_eq!(first["content"], json!(["comédie", "drama", "horror"]));
    assert_eq!(first["totalElements"], 4);
    assert_eq!(first["totalPages"], 2);
    assert_eq!(first["sort"]["sorted"], true);
    let last = page(&server, None, "/api/v2/series/release-years?size=2&page=1").await;
    assert_eq!(last["content"], json!(["1999"]));
    assert_eq!(last["totalElements"], 3);

    let anonymous = server.request(Method::GET, "/api/v2/genres").send().await;
    assert_eq!(
        anonymous.expect("Request").status(),
        StatusCode::UNAUTHORIZED
    );
    let bad = send(
        &server,
        None,
        Method::GET,
        "/api/v2/publishers?unpaged=x",
        None,
    )
    .await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn search_matches_parts_ignoring_accents_and_case() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, _) = catalog(directory.path()).await;
    let cases = [
        ("/api/v2/genres?search=come", json!(["comédie"])),
        ("/api/v2/genres?search=DRAMA", json!(["drama"])),
        ("/api/v2/genres?search=r", json!(["drama", "horror"])),
        (
            "/api/v2/genres?search=",
            json!(["comédie", "drama", "horror", "sci-fi"]),
        ),
        ("/api/v2/genres?search=%25", json!([])),
        ("/api/v2/genres?search=_", json!([])),
        ("/api/v2/publishers?search=acme", json!(["Ácme"])),
        ("/api/v2/languages?search=E", json!(["en"])),
        ("/api/v2/sharing-labels?search=ki", json!(["kids"])),
    ];
    for (path, expected) in cases {
        assert_eq!(values(&server, None, path).await, expected, "{path}");
    }
}

#[tokio::test]
async fn filters_and_user_scope_narrow_the_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, collection) = catalog(directory.path()).await;
    let scoped = json!({"sharedLibraries": {"all": false, "libraryIds": [library]}});
    create_user(&server, "comics@example.com", scoped).await;
    let no_adults = json!({"ageRestriction": {"age": 18, "restriction": "EXCLUDE"}});
    create_user(&server, "minor@example.com", no_adults).await;
    create_user(
        &server,
        "kids@example.com",
        json!({"labelsAllow": ["kids"]}),
    )
    .await;

    let cases = [
        (
            None,
            format!("/api/v2/genres?library_id={library}"),
            json!(["comédie", "drama", "horror"]),
        ),
        (
            None,
            "/api/v2/genres?library_id=other".to_owned(),
            json!([]),
        ),
        (
            None,
            format!("/api/v2/genres?collection_id={collection}"),
            json!(["comédie", "drama"]),
        ),
        (
            None,
            format!("/api/v2/genres?library_id=other&collection_id={collection}"),
            json!([]),
        ),
        (
            None,
            format!("/api/v2/series/release-years?collection_id={collection}"),
            json!(["1999"]),
        ),
        (
            Some("comics@example.com"),
            "/api/v2/genres".to_owned(),
            json!(["comédie", "drama", "horror"]),
        ),
        (
            Some("comics@example.com"),
            "/api/v2/series/release-years".to_owned(),
            json!(["2020", "1999"]),
        ),
        (
            Some("minor@example.com"),
            "/api/v2/genres".to_owned(),
            json!(["comédie", "drama", "sci-fi"]),
        ),
        (
            Some("minor@example.com"),
            "/api/v2/publishers".to_owned(),
            json!(["Pub One"]),
        ),
        (
            Some("minor@example.com"),
            "/api/v2/sharing-labels".to_owned(),
            json!(["kids"]),
        ),
        (
            Some("minor@example.com"),
            "/api/v2/age-ratings".to_owned(),
            json!([]),
        ),
        (
            Some("kids@example.com"),
            "/api/v2/languages".to_owned(),
            json!(["fr"]),
        ),
        (
            Some("kids@example.com"),
            "/api/v2/series/release-years".to_owned(),
            json!(["1999"]),
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
