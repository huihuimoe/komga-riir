//! Deprecated referential lists of `ReferentialV1Controller`.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{book_values, series_values};
use crate::read_lists::{create_user, send};
use crate::support::{TestServer, openapi};

/// The list `email` (else the admin) gets from `path`.
async fn list(server: &TestServer, email: Option<&str>, path: &str) -> Value {
    let response = send(server, email, Method::GET, path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    openapi::checked(Method::GET, response).await
}

async fn create_minor(server: &TestServer) -> Option<&'static str> {
    let no_adults = json!({"ageRestriction": {"age": 18, "restriction": "EXCLUDE"}});
    create_user(server, "minor@example.com", no_adults).await;
    Some("minor@example.com")
}

#[tokio::test]
async fn series_values_list_unpaged() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, collection) = series_values::catalog(directory.path()).await;
    let minor = create_minor(&server).await;
    let cases = [
        (
            None,
            "/api/v1/genres".to_owned(),
            json!(["comédie", "drama", "horror", "sci-fi"]),
        ),
        (
            None,
            "/api/v1/sharing-labels".to_owned(),
            json!(["adult", "kids"]),
        ),
        (None, "/api/v1/languages".to_owned(), json!(["en", "fr"])),
        (
            None,
            "/api/v1/publishers".to_owned(),
            json!(["Ácme", "Pub One"]),
        ),
        (
            None,
            "/api/v1/age-ratings".to_owned(),
            json!(["None", "18"]),
        ),
        (
            None,
            "/api/v1/series/release-dates".to_owned(),
            json!(["2020", "2005", "1999"]),
        ),
        (
            None,
            format!("/api/v1/genres?library_id={library}"),
            json!(["comédie", "drama", "horror"]),
        ),
        (
            None,
            format!("/api/v1/age-ratings?library_id={library},other"),
            json!(["None", "18"]),
        ),
        (
            None,
            format!("/api/v1/publishers?collection_id={collection}"),
            json!(["Pub One"]),
        ),
        (
            None,
            format!("/api/v1/series/release-dates?collection_id={collection}"),
            json!(["1999"]),
        ),
        (
            minor,
            "/api/v1/genres".to_owned(),
            json!(["comédie", "drama", "sci-fi"]),
        ),
        (minor, "/api/v1/age-ratings".to_owned(), json!(["None"])),
        (
            minor,
            "/api/v1/series/release-dates".to_owned(),
            json!(["2005", "1999"]),
        ),
    ];
    for (email, path, expected) in cases {
        assert_eq!(
            list(&server, email, &path).await,
            expected,
            "{email:?} {path}"
        );
    }
    let anonymous = server.request(Method::GET, "/api/v1/genres").send().await;
    assert_eq!(
        anonymous.expect("Request").status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn book_values_list_unpaged() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, catalog) = book_values::catalog(directory.path()).await;
    let book_values::Catalog {
        library,
        alpha,
        read_list,
        collection,
    } = &catalog;
    let minor = create_minor(&server).await;
    let author = book_values::author;
    let cases = [
        (
            None,
            "/api/v1/authors".to_owned(),
            json!([
                author("Ann", "writer"),
                author("Bób", "penciller"),
                author("Cy", "colorist"),
                author("Zed", "writer"),
            ]),
        ),
        (
            None,
            format!("/api/v1/authors?series_id={alpha}&search=n"),
            json!([author("Ann", "writer")]),
        ),
        (
            None,
            format!("/api/v1/authors?collection_id={collection}&series_id={alpha}"),
            json!([author("Ann", "writer"), author("Cy", "colorist")]),
        ),
        (
            None,
            format!("/api/v1/authors?library_id={library},x"),
            json!([]),
        ),
        (
            None,
            "/api/v1/authors/names?search=B".to_owned(),
            json!(["Bób"]),
        ),
        (
            None,
            "/api/v1/authors/roles".to_owned(),
            json!(["colorist", "penciller", "writer"]),
        ),
        (
            None,
            "/api/v1/tags".to_owned(),
            json!(["adult tag", "book tag", "series tag"]),
        ),
        (
            None,
            format!("/api/v1/tags?collection_id={collection}"),
            json!(["adult tag"]),
        ),
        (
            None,
            format!("/api/v1/tags/book?readlist_id={read_list}&library_id={library}"),
            json!(["adult tag"]),
        ),
        (
            None,
            format!("/api/v1/tags/book?library_id={library}"),
            json!(["adult tag", "book tag"]),
        ),
        (
            None,
            format!("/api/v1/tags/series?library_id={library}"),
            json!(["series tag"]),
        ),
        (
            None,
            format!("/api/v1/tags/series?collection_id={collection}"),
            json!([]),
        ),
        (
            minor,
            "/api/v1/authors/names".to_owned(),
            json!(["Ann", "Bób", "Zed"]),
        ),
        (minor, "/api/v1/tags/book".to_owned(), json!(["book tag"])),
    ];
    for (email, path, expected) in cases {
        assert_eq!(
            list(&server, email, &path).await,
            expected,
            "{email:?} {path}"
        );
    }
}
