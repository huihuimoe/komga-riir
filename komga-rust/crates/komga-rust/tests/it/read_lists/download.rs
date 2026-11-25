//! `GET /api/v1/readlists/{id}/file`.

use std::fs;

use reqwest::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde_json::json;

use super::{READ_LISTS, create_user, fixture, send};
use crate::support::cbz::entries;
use crate::support::json;

#[tokio::test]
async fn archives_hold_the_visible_files_named_by_list_number() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let body = json!({"name": "Arc é", "bookIds": [&f.b1, &f.a2, &f.a1]});
    let created = json(send(&f.server, None, Method::POST, READ_LISTS, Some(&body)).await).await;
    let path = format!("{READ_LISTS}/{}/file", created["id"].as_str().expect("ID"));
    let book = fs::read(directory.path().join("a/SA/a1.cbz")).expect("Book file");

    let response = send(&f.server, None, Method::GET, &path, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/zip");
    assert_eq!(
        response.headers()[CONTENT_DISPOSITION],
        "attachment; filename=\"=?UTF-8?Q?Arc_=C3=A9.zip?=\"; filename*=UTF-8''Arc%20%C3%A9.zip"
    );
    let archive = response.bytes().await.expect("Archive").to_vec();
    let expected = [
        ("1 - b1.cbz", &book),
        ("2 - a2.cbz", &book),
        ("3 - a1.cbz", &book),
    ];
    let expected: Vec<(String, Vec<u8>)> = expected
        .iter()
        .map(|(name, content)| ((*name).to_owned(), (*content).clone()))
        .collect();
    assert!(entries(archive) == expected, "every book in list order");

    // Numbers stay those of the list when some books are hidden, and missing files are skipped.
    let library_a = json!({"roles": ["FILE_DOWNLOAD"], "sharedLibraries": {
        "all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.org", library_a).await;
    fs::remove_file(directory.path().join("a/SA/a2.cbz")).expect("Remove a book file");
    let response = send(&f.server, Some("a@example.org"), Method::GET, &path, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let archive = response.bytes().await.expect("Archive").to_vec();
    assert!(
        entries(archive) == [("3 - a1.cbz".to_owned(), book)],
        "only the visible book with a file"
    );
}

#[tokio::test]
async fn downloads_need_file_download_and_a_visible_list() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let body = json!({"name": "B only", "bookIds": [&f.b1]});
    let created = json(send(&f.server, None, Method::POST, READ_LISTS, Some(&body)).await).await;
    let path = format!("{READ_LISTS}/{}/file", created["id"].as_str().expect("ID"));
    let missing = format!("{READ_LISTS}/missing/file");

    create_user(&f.server, "reader@example.org", json!({"roles": []})).await;
    for path in [&path, &missing] {
        let response = send(
            &f.server,
            Some("reader@example.org"),
            Method::GET,
            path,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        assert_eq!(json(response).await["message"], "Forbidden", "{path}");
    }
    let library_a = json!({"roles": ["FILE_DOWNLOAD"], "sharedLibraries": {
        "all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.org", library_a).await;
    let response = send(&f.server, Some("a@example.org"), Method::GET, &path, None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = send(&f.server, None, Method::GET, &missing, None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
