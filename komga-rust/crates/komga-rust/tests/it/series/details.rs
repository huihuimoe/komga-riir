//! `GET /api/v1/series/{id}`.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::books::{PNG, library_with_books};
use crate::read_lists::{create_user, rate, send};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

/// A book of two pages and the given `ComicInfo.xml`.
pub(crate) fn comic(comic_info: &str) -> Vec<u8> {
    let png = cbz::fixture(PNG);
    cbz::zip(&[
        ("1.png", STORED, &png),
        ("2.png", STORED, &png),
        ("ComicInfo.xml", STORED, comic_info.as_bytes()),
    ])
}

/// The series as `email` (else the admin) sees it, or the failure status.
async fn get(server: &TestServer, email: Option<&str>, id: &str) -> Result<Value, StatusCode> {
    let response = send(
        server,
        email,
        Method::GET,
        &format!("/api/v1/series/{id}"),
        None,
    )
    .await;
    match response.status() {
        StatusCode::OK => Ok(openapi::checked(Method::GET, response).await),
        status => Err(status),
    }
}

pub(crate) async fn mark(server: &TestServer, book_id: &str, body: Value) {
    let path = format!("/api/v1/books/{book_id}/read-progress");
    let response = send(server, None, Method::PATCH, &path, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn series_show_their_metadata_and_the_read_counts_of_the_user() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = [
        (
            "Series/a.cbz",
            comic(
                "<ComicInfo><Number>1</Number><Summary>First</Summary><Year>2020</Year>\
                 <Month>5</Month><Writer>Ann</Writer><Characters>Eve</Characters>\
                 <Tags>x</Tags></ComicInfo>",
            ),
        ),
        (
            "Series/b.cbz",
            comic("<ComicInfo><Number>2</Number><Tags>y</Tags></ComicInfo>"),
        ),
        (
            "Series/c.cbz",
            comic("<ComicInfo><Number>3</Number></ComicInfo>"),
        ),
    ];
    let (server, library, pool) = library_with_books(directory.path(), &books).await;
    let (a, series_id) = book(&pool, "a").await;
    let (b, _) = book(&pool, "b").await;
    mark(&server, &a, json!({"completed": true})).await;
    mark(&server, &b, json!({"page": 1})).await;

    let series = get(&server, None, &series_id).await.expect("Series");
    assert_eq!(series["id"], series_id.as_str());
    assert_eq!(series["libraryId"], library["id"]);
    assert_eq!(series["name"], "Series");
    let root = directory.path().join("comics").join("Series");
    assert_eq!(series["url"], root.to_string_lossy().as_ref());
    let counts = [
        "booksCount",
        "booksReadCount",
        "booksUnreadCount",
        "booksInProgressCount",
    ]
    .map(|key| series[key].as_i64().expect("Count"));
    assert_eq!(counts, [3, 1, 1, 1]);
    assert_eq!(series["metadata"]["title"], "Series");
    assert_eq!(series["metadata"]["readingDirection"], "");
    assert_eq!(series["metadata"]["ageRating"], Value::Null);
    let books_metadata = &series["booksMetadata"];
    assert_eq!(
        books_metadata["authors"],
        json!([{"name": "Ann", "role": "writer"}])
    );
    assert_eq!(
        books_metadata["notableEntities"],
        json!([{"kind": "character", "name": "Eve"}])
    );
    let mut tags: Vec<&str> = books_metadata["tags"]
        .as_array()
        .expect("Tags")
        .iter()
        .map(|tag| tag.as_str().expect("Tag"))
        .collect();
    tags.sort_unstable();
    assert_eq!(tags, ["x", "y"]);
    assert_eq!(books_metadata["releaseDate"], "2020-05-01");
    assert_eq!(books_metadata["summary"], "First");
    assert_eq!(books_metadata["summaryNumber"], "1");
    assert_eq!(series["deleted"], false);
    assert_eq!(series["oneshot"], false);
    assert_eq!(
        get(&server, None, "missing").await,
        Err(StatusCode::NOT_FOUND)
    );

    let sixteen = json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    create_user(&server, "reader@example.com", sixteen).await;
    let seen = get(&server, Some("reader@example.com"), &series_id)
        .await
        .expect("Series");
    assert_eq!(seen["url"], "", "only admins see the path of the directory");
    let counts = ["booksReadCount", "booksUnreadCount", "booksInProgressCount"]
        .map(|key| seen[key].as_i64().expect("Count"));
    assert_eq!(counts, [0, 3, 0], "read counts are the user's own");
    rate(&server, &series_id, 18).await;
    let hidden = get(&server, Some("reader@example.com"), &series_id).await;
    assert_eq!(hidden, Err(StatusCode::FORBIDDEN));
    let missing = get(&server, Some("reader@example.com"), "missing").await;
    assert_eq!(missing, Err(StatusCode::NOT_FOUND));

    let elsewhere = json!({"sharedLibraries": {"all": false, "libraryIds": []}});
    create_user(&server, "elsewhere@example.com", elsewhere).await;
    let hidden = get(&server, Some("elsewhere@example.com"), &series_id).await;
    assert_eq!(hidden, Err(StatusCode::FORBIDDEN));
    let unauthenticated = server
        .request(Method::GET, &format!("/api/v1/series/{series_id}"))
        .send()
        .await
        .expect("Request");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    server.stop().await;
}
