//! `GET /api/v1/books/{id}/readlists`.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{Fixture, create_list, create_user, fixture, rate, send};
use crate::support::openapi;

/// The read lists of `book` as `email` sees them, or the failure status.
async fn of_book(fixture: &Fixture, email: Option<&str>, book: &str) -> Result<Value, StatusCode> {
    let path = format!("/api/v1/books/{book}/readlists");
    let response = send(&fixture.server, email, Method::GET, &path, None).await;
    match response.status() {
        StatusCode::OK => Ok(openapi::checked(Method::GET, response).await),
        status => Err(status),
    }
}

#[tokio::test]
async fn read_lists_of_a_book_show_only_visible_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let both = create_list(&f, "Both", &[&f.b1, &f.a1]).await;
    let only_a = create_list(&f, "Only A", &[&f.a1, &f.a2]).await;
    create_list(&f, "Only B", &[&f.b1]).await;

    let lists = of_book(&f, None, &f.a1).await.expect("Read lists");
    let mut ids: Vec<&str> = lists
        .as_array()
        .expect("Read list array")
        .iter()
        .map(|list| list["id"].as_str().expect("Read list ID"))
        .collect();
    ids.sort_unstable();
    let mut expected = [both.as_str(), only_a.as_str()];
    expected.sort_unstable();
    assert_eq!(ids, expected);
    assert_eq!(
        of_book(&f, None, &f.a2)
            .await
            .expect("Read lists")
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(of_book(&f, None, "missing").await, Ok(json!([])));

    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.com", library_a).await;
    let seen = of_book(&f, Some("a@example.com"), &f.a1)
        .await
        .expect("Read lists");
    let both_seen = seen
        .as_array()
        .expect("Read list array")
        .iter()
        .find(|list| list["id"] == both.as_str())
        .expect("Both list");
    assert_eq!(both_seen["bookIds"], json!([&f.a1]));
    assert_eq!(both_seen["filtered"], true);
    let hidden = of_book(&f, Some("a@example.com"), &f.b1).await;
    assert_eq!(hidden, Err(StatusCode::FORBIDDEN));
    let missing = of_book(&f, Some("a@example.com"), "missing").await;
    assert_eq!(missing, Err(StatusCode::NOT_FOUND));

    let allow_12 = json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}});
    create_user(&f.server, "twelve@example.com", allow_12).await;
    rate(&f.server, &f.series_b, 6).await;
    assert_eq!(
        of_book(&f, Some("twelve@example.com"), &f.a1).await,
        Err(StatusCode::FORBIDDEN)
    );
    let seen = of_book(&f, Some("twelve@example.com"), &f.b1)
        .await
        .expect("Read lists");
    assert_eq!(seen.as_array().map(Vec::len), Some(2));
    f.server.stop().await;
}
