//! The deprecated `GET /api/v1/books` and `GET /api/v1/series/{id}/books`
//! (`BookController.getAllBooksDeprecated`, `SeriesController.getBooksBySeriesId`).

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::list::{edit_metadata, two_series};
use super::read_progress::mark;
use crate::read_lists::{create_user, rate, send};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

const RESTRICTED: &str = "restricted@example.com";

async fn page(server: &TestServer, email: Option<&str>, path: &str) -> Value {
    let response = send(server, email, Method::GET, path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    openapi::checked(Method::GET, response).await
}

async fn status(server: &TestServer, email: Option<&str>, path: &str) -> StatusCode {
    send(server, email, Method::GET, path, None).await.status()
}

fn names(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|book| book["name"].as_str().expect("Book name"))
        .collect()
}

/// Names for `query` on `GET /api/v1/books`, by name.
async fn books(server: &TestServer, query: &str) -> Vec<String> {
    let path = format!("/api/v1/books?unpaged=true&sort=name&{query}");
    let page = page(server, None, &path).await;
    names(&page).into_iter().map(str::to_owned).collect()
}

#[tokio::test]
async fn query_parameters_filter_all_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = two_series(directory.path()).await;
    let (a1, _) = book(&pool, "a1").await;
    mark(&server, &a1, json!({"completed": true})).await;
    edit_metadata(
        &server,
        &book(&pool, "b1").await.0,
        json!({"tags": ["Hero"]}),
    )
    .await;
    let b2 = book(&pool, "b2").await.0;
    edit_metadata(&server, &b2, json!({"releaseDate": "2020-06-01"})).await;
    let library_id = library["id"].as_str().expect("Library ID");

    let all = page(&server, None, "/api/v1/books").await;
    assert_eq!(all["totalElements"], 5);
    assert_eq!(all["sort"]["sorted"], false);
    let by_name = page(&server, None, "/api/v1/books?sort=name,desc&size=2").await;
    assert_eq!(names(&by_name), ["b2", "b1"]);
    assert_eq!(by_name["sort"]["sorted"], true);
    let read_list = page(&server, None, "/api/v1/books?sort=readList.number").await;
    assert_eq!(read_list["sort"]["sorted"], false);
    let everything = ["a1", "a2", "a3", "b1", "b2"];
    for (query, expected) in [
        (format!("library_id={library_id}"), everything.to_vec()),
        ("library_id=0".to_owned(), vec![]),
        ("read_status=READ".to_owned(), vec!["a1"]),
        (
            "read_status=UNREAD,IN_PROGRESS".to_owned(),
            vec!["a2", "a3", "b1", "b2"],
        ),
        ("media_status=READY".to_owned(), everything.to_vec()),
        (
            "media_status=ERROR&media_status=OUTDATED".to_owned(),
            vec![],
        ),
        ("tag=Hero".to_owned(), vec!["b1"]),
        ("released_after=2020-05-31".to_owned(), vec!["b2"]),
        ("released_after=2020-06-02".to_owned(), vec![]),
        ("released_after=".to_owned(), everything.to_vec()),
        // Not a parameter of this route.
        (
            "author=Ann,writer&deleted=true".to_owned(),
            everything.to_vec(),
        ),
    ] {
        assert_eq!(books(&server, &query).await, expected, "{query}");
    }
    for query in [
        "media_status=BAD",
        "released_after=2020-13-01",
        "unpaged=maybe",
    ] {
        let path = format!("/api/v1/books?{query}");
        assert_eq!(
            status(&server, None, &path).await,
            StatusCode::BAD_REQUEST,
            "{query}"
        );
    }

    let (_, beta) = book(&pool, "b1").await;
    rate(&server, &beta, 18).await;
    let sixteen = json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    create_user(&server, RESTRICTED, sixteen).await;
    let seen = page(&server, Some(RESTRICTED), "/api/v1/books?sort=name").await;
    assert_eq!(names(&seen), ["a1", "a2", "a3"]);
    assert_eq!(seen["content"][0]["url"], "a1.cbz");
    assert_eq!(status(&server, None, "/api/v1/books").await, StatusCode::OK);
}

#[tokio::test]
async fn series_books_come_by_number_within_the_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = two_series(directory.path()).await;
    let (a2, alpha) = book(&pool, "a2").await;
    let (b1, beta) = book(&pool, "b1").await;
    edit_metadata(
        &server,
        &a2,
        json!({"authors": [{"name": "Ann", "role": "writer"}]}),
    )
    .await;
    mark(&server, &b1, json!({"completed": true})).await;
    let path = |query: &str| format!("/api/v1/series/{alpha}/books{query}");

    let books = page(&server, None, &path("")).await;
    assert_eq!(names(&books), ["a1", "a2", "a3"]);
    assert_eq!(books["sort"]["sorted"], true);
    assert_eq!(
        names(&page(&server, None, &path("?sort=name,desc")).await),
        ["a3", "a2", "a1"]
    );
    let second = page(&server, None, &path("?size=2&page=1")).await;
    assert_eq!(names(&second), ["a3"]);
    assert_eq!(second["totalElements"], 3);
    for (query, expected) in [
        ("?author=Ann,writer", vec!["a2"]),
        ("?author=Ann,penciller", vec![]),
        ("?read_status=READ", vec![]),
        ("?deleted=true", vec![]),
        ("?deleted=false&media_status=READY", vec!["a1", "a2", "a3"]),
        // Not a parameter of this route.
        ("?library_id=0", vec!["a1", "a2", "a3"]),
    ] {
        assert_eq!(
            names(&page(&server, None, &path(query)).await),
            expected,
            "{query}"
        );
    }
    let beta_books = format!("/api/v1/series/{beta}/books?read_status=READ");
    assert_eq!(names(&page(&server, None, &beta_books).await), ["b1"]);
    let unknown = page(&server, None, "/api/v1/series/0/books").await;
    assert_eq!(unknown["totalElements"], 0);

    rate(&server, &beta, 18).await;
    let sixteen = json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    create_user(&server, RESTRICTED, sixteen).await;
    let hidden = format!("/api/v1/series/{beta}/books");
    assert_eq!(
        status(&server, Some(RESTRICTED), &hidden).await,
        StatusCode::FORBIDDEN
    );
    let missing = "/api/v1/series/0/books";
    assert_eq!(
        status(&server, Some(RESTRICTED), missing).await,
        StatusCode::NOT_FOUND
    );
    let seen = page(&server, Some(RESTRICTED), &path("")).await;
    assert_eq!(seen["totalElements"], 3);
    let anonymous = server
        .request(Method::GET, &hidden)
        .send()
        .await
        .expect("Request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}
