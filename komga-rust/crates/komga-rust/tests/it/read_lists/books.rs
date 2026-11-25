//! `GET /api/v1/readlists/{id}/books` and `…/books/{bookId}/previous|next`.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{Fixture, create_list, create_user, fixture, rate, send};
use crate::support::openapi;

/// The page of books of read list `id` for `query`, as `email` sees it, or the failure status.
async fn books(
    f: &Fixture,
    email: Option<&str>,
    id: &str,
    query: &str,
) -> Result<Value, StatusCode> {
    let path = format!("/api/v1/readlists/{id}/books{query}");
    let response = send(&f.server, email, Method::GET, &path, None).await;
    match response.status() {
        StatusCode::OK => Ok(openapi::checked(Method::GET, response).await),
        status => Err(status),
    }
}

/// The IDs of the books of a page, in order.
fn ids(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|book| book["id"].as_str().expect("Book ID"))
        .collect()
}

/// The ID of the neighbour (`previous` or `next`) of `book` in read list `id`, or the failure.
async fn sibling(
    f: &Fixture,
    email: Option<&str>,
    id: &str,
    book: &str,
    which: &str,
) -> Result<String, StatusCode> {
    let path = format!("/api/v1/readlists/{id}/books/{book}/{which}");
    let response = send(&f.server, email, Method::GET, &path, None).await;
    match response.status() {
        StatusCode::OK => {
            let found: Value = openapi::checked(Method::GET, response).await;
            Ok(found["id"].as_str().expect("Book ID").to_owned())
        }
        status => Err(status),
    }
}

async fn edit_metadata(f: &Fixture, book: &str, body: Value) {
    let path = format!("/api/v1/books/{book}/metadata");
    let response = send(&f.server, None, Method::PATCH, &path, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn ordered_lists_page_and_step_by_number() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let list = create_list(&f, "Mixed", &[&f.a2, &f.b1, &f.a1]).await;

    let all = books(&f, None, &list, "").await.expect("Books");
    assert_eq!(ids(&all), [&f.a2, &f.b1, &f.a1]);
    assert_eq!(all["sort"]["sorted"], true);
    assert_eq!(
        all["content"][0]["url"]
            .as_str()
            .map(|url| url.ends_with("a2.cbz")),
        Some(true)
    );
    let second = books(&f, None, &list, "?size=1&page=1&sort=name,desc")
        .await
        .expect("Books");
    assert_eq!(ids(&second), [&f.b1]);
    assert_eq!(second["totalElements"], 3);
    let unpaged = books(&f, None, &list, "?unpaged=true&size=1")
        .await
        .expect("Books");
    assert_eq!(ids(&unpaged).len(), 3);

    assert_eq!(
        sibling(&f, None, &list, &f.b1, "previous").await,
        Ok(f.a2.clone())
    );
    assert_eq!(
        sibling(&f, None, &list, &f.b1, "next").await,
        Ok(f.a1.clone())
    );
    let before_first = sibling(&f, None, &list, &f.a2, "previous").await;
    assert_eq!(before_first, Err(StatusCode::NOT_FOUND));
    let after_last = sibling(&f, None, &list, &f.a1, "next").await;
    assert_eq!(after_last, Err(StatusCode::NOT_FOUND));
    let outside = create_list(&f, "Other", &[&f.a1]).await;
    let not_member = sibling(&f, None, &outside, &f.b1, "next").await;
    assert_eq!(not_member, Err(StatusCode::NOT_FOUND));
    assert_eq!(
        books(&f, None, "missing", "").await,
        Err(StatusCode::NOT_FOUND)
    );
    let unknown = sibling(&f, None, "missing", &f.a1, "next").await;
    assert_eq!(unknown, Err(StatusCode::NOT_FOUND));
    f.server.stop().await;
}

#[tokio::test]
async fn unordered_lists_follow_release_dates() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let list = create_list(&f, "Dated", &[&f.b1, &f.a1, &f.a2]).await;
    let path = format!("/api/v1/readlists/{list}");
    let body = json!({"ordered": false});
    let response = send(&f.server, None, Method::PATCH, &path, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    edit_metadata(&f, &f.b1, json!({"releaseDate": "2021-01-01"})).await;
    edit_metadata(&f, &f.a1, json!({"releaseDate": "2020-01-01"})).await;

    // No release date sorts first; the date then the list number order the rest.
    let all = books(&f, None, &list, "").await.expect("Books");
    assert_eq!(ids(&all), [&f.a2, &f.a1, &f.b1]);
    assert_eq!(
        sibling(&f, None, &list, &f.a1, "previous").await,
        Ok(f.a2.clone())
    );
    assert_eq!(
        sibling(&f, None, &list, &f.a1, "next").await,
        Ok(f.b1.clone())
    );

    edit_metadata(&f, &f.a1, json!({"releaseDate": "2021-01-01"})).await;
    let tied = books(&f, None, &list, "").await.expect("Books");
    assert_eq!(ids(&tied), [&f.a2, &f.b1, &f.a1]);
    assert_eq!(
        sibling(&f, None, &list, &f.b1, "next").await,
        Ok(f.a1.clone())
    );
    f.server.stop().await;
}

#[tokio::test]
async fn filter_parameters_match_any_of_their_values() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let list = create_list(&f, "Mixed", &[&f.a2, &f.b1, &f.a1]).await;
    let authors =
        json!([{"name": "Ann, Jr", "role": "writer"}, {"name": "Bo", "role": "penciller"}]);
    edit_metadata(&f, &f.a1, json!({"tags": ["x"], "authors": authors})).await;
    edit_metadata(&f, &f.b1, json!({"tags": ["y"]})).await;
    let path = format!("/api/v1/books/{}/read-progress", f.a2);
    let read = json!({"completed": true});
    let response = send(&f.server, None, Method::PATCH, &path, Some(&read)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let filtered = |query: &'static str| {
        let f = &f;
        let list = &list;
        async move { books(f, None, list, query).await.expect("Books") }
    };
    let library = format!("?library_id={}", f.library_a);
    let in_a = books(&f, None, &list, &library).await.expect("Books");
    assert_eq!(ids(&in_a), [&f.a2, &f.a1]);
    assert_eq!(ids(&filtered("?read_status=READ").await), [&f.a2]);
    let unread = filtered("?read_status=UNREAD,IN_PROGRESS").await;
    assert_eq!(ids(&unread), [&f.b1, &f.a1]);
    assert_eq!(ids(&filtered("?media_status=READY").await).len(), 3);
    assert_eq!(ids(&filtered("?media_status=ERROR").await).len(), 0);
    assert_eq!(ids(&filtered("?tag=x&tag=y").await), [&f.b1, &f.a1]);
    assert_eq!(ids(&filtered("?tag=x,z").await), [&f.a1]);
    assert_eq!(ids(&filtered("?author=Ann,%20Jr,Writer").await), [&f.a1]);
    assert_eq!(ids(&filtered("?author=Ann,%20Jr,penciller").await).len(), 0);
    assert_eq!(ids(&filtered("?author=").await).len(), 3);
    assert_eq!(ids(&filtered("?deleted=false").await).len(), 3);
    assert_eq!(ids(&filtered("?deleted=true").await).len(), 0);
    assert_eq!(ids(&filtered("?deleted=&tag=").await).len(), 3);

    for query in ["?read_status=DONE", "?media_status=ready", "?deleted=maybe"] {
        assert_eq!(
            books(&f, None, &list, query).await,
            Err(StatusCode::BAD_REQUEST),
            "{query}"
        );
    }
    f.server.stop().await;
}

#[tokio::test]
async fn restricted_users_see_only_their_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let list = create_list(&f, "Mixed", &[&f.a2, &f.b1, &f.a1]).await;
    let only_b = create_list(&f, "Only B", &[&f.b1]).await;
    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    let a = Some("a@example.com");
    create_user(&f.server, "a@example.com", library_a).await;

    let seen = books(&f, a, &list, "").await.expect("Books");
    assert_eq!(ids(&seen), [&f.a2, &f.a1]);
    assert_eq!(seen["totalElements"], 2);
    assert_eq!(seen["content"][0]["url"], "a2.cbz");
    assert_eq!(sibling(&f, a, &list, &f.a2, "next").await, Ok(f.a1.clone()));
    assert_eq!(
        sibling(&f, a, &list, &f.b1, "next").await,
        Err(StatusCode::NOT_FOUND)
    );
    assert_eq!(books(&f, a, &only_b, "").await, Err(StatusCode::NOT_FOUND));
    let hidden_list = sibling(&f, a, &only_b, &f.b1, "previous").await;
    assert_eq!(hidden_list, Err(StatusCode::NOT_FOUND));

    let allow_12 = json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}});
    let twelve = Some("twelve@example.com");
    create_user(&f.server, "twelve@example.com", allow_12).await;
    rate(&f.server, &f.series_b, 6).await;
    let rated = books(&f, twelve, &list, "").await.expect("Books");
    assert_eq!(ids(&rated), [&f.b1]);
    let hidden = sibling(&f, twelve, &list, &f.a2, "next").await;
    assert_eq!(hidden, Err(StatusCode::NOT_FOUND));
    let unauthenticated = f
        .server
        .request(Method::GET, &format!("/api/v1/readlists/{list}/books"))
        .send()
        .await
        .expect("Request");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    f.server.stop().await;
}
