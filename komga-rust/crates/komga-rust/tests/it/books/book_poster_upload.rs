//! `POST /api/v1/books/{bookId}/thumbnails`: uploading book posters.

use std::fs;

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::multipart::{Form, Part};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::book_posters::{id_of, list, poster, route, two_books};
use super::library_with_books;
use super::reading::access::create_reader;
use super::reading::fetch;
use super::thumbnails::{book_with_first_page, cover};
use crate::libraries;
use crate::support::db::{self, content::book};
use crate::support::posters::{file, png, selected_ids, status_and_body};
use crate::support::tasks::{wait_for_generations, wait_for_scan_work, wait_for_tasks};
use crate::support::{TestServer, openapi};

async fn uploaded(server: &TestServer, path: &str, form: Form) -> Value {
    let response = server
        .admin(Method::POST, path)
        .multipart(form)
        .send()
        .await
        .expect("Upload request");
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::POST, response).await
}

#[tokio::test]
async fn uploads_are_selected_on_request_and_deletable() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, a, _) = two_books(directory.path()).await;
    let first = png(120, 80);

    // `selected` defaults to true.
    let dto = uploaded(&server, &route(&a), file(first.clone())).await;
    assert_eq!(
        dto,
        json!({
            "id": dto["id"], "bookId": a, "type": "USER_UPLOADED", "selected": true,
            "mediaType": "image/png", "fileSize": first.len(), "width": 120, "height": 80,
        })
    );
    let first_id = dto["id"].as_str().expect("Poster ID").to_owned();
    assert_eq!(selected_ids(&list(&server, &a).await), [first_id.as_str()]);
    let served = fetch(server.admin(Method::GET, &format!("/api/v1/books/{a}/thumbnail"))).await;
    assert!(
        served.body == first,
        "the uploaded bytes are the book thumbnail"
    );

    // An unselected upload, with `selected` from the query string, keeps the selection.
    let path = format!("{}?selected=false", route(&a));
    let dto = uploaded(&server, &path, file(png(60, 40))).await;
    assert_eq!(dto["selected"], false);
    let second_id = dto["id"].as_str().expect("Poster ID").to_owned();
    assert_eq!(selected_ids(&list(&server, &a).await), [first_id.as_str()]);

    // Deleting the selected upload selects another poster.
    for id in [&first_id, &second_id] {
        let response = server
            .admin(Method::DELETE, &format!("{}/{id}", route(&a)))
            .send()
            .await
            .expect("Delete request");
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let posters = list(&server, &a).await;
        let selected = selected_ids(&posters);
        assert_eq!(selected.len(), 1, "{posters:?}");
        assert!(!posters.iter().any(|poster| poster["id"] == id.as_str()));
    }
    assert_eq!(list(&server, &a).await.len(), 2, "sidecar and generated");
    server.stop().await;
}

/// Neither regenerating thumbnails nor a scan that finds the book changed and its sidecar gone
/// drops or deselects an uploaded poster.
#[tokio::test]
async fn uploads_survive_regeneration_and_scans() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = library_with_books(
        directory.path(),
        &[
            ("A/a.cbz", book_with_first_page(&cover())),
            ("A/a.png", cover()),
        ],
    )
    .await;
    wait_for_generations(directory.path()).await;
    let (a, _) = book(&pool, "a").await;
    let form = file(png(120, 80)).text("selected", "true");
    let upload_id = uploaded(&server, &route(&a), form).await["id"].clone();

    let response = server
        .admin(Method::PUT, "/api/v1/books/thumbnails")
        .send()
        .await
        .expect("Regeneration request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    wait_for_tasks(directory.path(), |rows| {
        rows.iter().all(|row| {
            !row.2.ends_with("$FindBookThumbnailsToRegenerate")
                && !row.2.ends_with("$GenerateBookThumbnail")
        })
    })
    .await;

    let root = directory.path().join("comics/A");
    fs::write(root.join("a.cbz"), book_with_first_page(&png(500, 700))).expect("Replace book");
    fs::remove_file(root.join("a.png")).expect("Remove sidecar");
    let response = server
        .admin(
            Method::POST,
            &format!("{}/scan?deep=true", libraries::route(&library)),
        )
        .send()
        .await
        .expect("Scan request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    wait_for_scan_work(directory.path()).await;
    wait_for_generations(directory.path()).await;

    let posters = list(&server, &a).await;
    assert_eq!(
        posters.len(),
        2,
        "upload and regenerated poster: {posters:?}"
    );
    assert_eq!(poster(&posters, "GENERATED")["height"], 300);
    assert_eq!(
        id_of(&posters, "USER_UPLOADED"),
        upload_id.as_str().expect("ID")
    );
    assert_eq!(selected_ids(&posters), [upload_id.as_str().expect("ID")]);
    server.stop().await;
}

#[tokio::test]
async fn invalid_uploads_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, a, _) = two_books(directory.path()).await;
    create_reader(&server, "reader@example.com", json!({"roles": []})).await;
    let as_reader = || {
        server
            .request(Method::POST, &route(&a))
            .basic_auth("reader@example.com", Some("secret"))
    };
    let admin = || server.admin(Method::POST, &route(&a));
    let missing = "Required part 'file' is not present.";
    let not_image = "415 UNSUPPORTED_MEDIA_TYPE";
    let cases = [
        (
            server.request(Method::POST, &route(&a)),
            StatusCode::UNAUTHORIZED,
            None,
        ),
        // `file` and `selected` bind before `@PreAuthorize` runs.
        (
            as_reader().multipart(Form::new().text("selected", "true")),
            StatusCode::BAD_REQUEST,
            Some(missing),
        ),
        (
            as_reader().multipart(file(png(8, 8)).text("selected", "maybe")),
            StatusCode::BAD_REQUEST,
            None,
        ),
        (
            as_reader().multipart(file(png(8, 8))),
            StatusCode::FORBIDDEN,
            Some("Forbidden"),
        ),
        // A field without a file name is no file.
        (
            admin().multipart(Form::new().text("file", "poster")),
            StatusCode::BAD_REQUEST,
            Some(missing),
        ),
        (
            server
                .admin(Method::POST, &route("missing"))
                .multipart(file(png(8, 8))),
            StatusCode::NOT_FOUND,
            None,
        ),
        (
            admin().multipart(file(b"not an image".to_vec())),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Some(not_image),
        ),
    ];
    for (request, status, message) in cases {
        let (actual, body) = status_and_body(request).await;
        assert_eq!(actual, status, "{body}");
        if let Some(message) = message {
            assert_eq!(body["message"], message);
        }
    }

    let response = admin().json(&json!({})).send().await.expect("JSON upload");
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(response.headers()[ACCEPT], "multipart/form-data");
    let body: Value = response.json().await.expect("Error body");
    assert_eq!(
        body["message"],
        "Content-Type 'application/json' is not supported."
    );
    assert_eq!(list(&server, &a).await.len(), 2, "nothing stored");
    server.stop().await;
}

#[tokio::test]
async fn oversized_uploads_are_payload_too_large() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, a, _) = two_books(directory.path()).await;
    let part = vec![0; 1024 * 1024];
    let mut too_many = Form::new();
    for index in 0..10 {
        too_many = too_many.text(format!("field{index}"), "x".repeat(1024 * 1024 - 1));
    }
    let too_many = too_many.part("file", Part::bytes(png(8, 8)).file_name("p.png"));
    let mut big = part.clone();
    big.push(0);
    for form in [
        file(big),
        Form::new().text("selected", "x".repeat(part.len() + 1)),
        too_many,
    ] {
        let response = server
            .admin(Method::POST, &route(&a))
            .multipart(form)
            .send()
            .await
            .expect("Upload request");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/problem+json");
        let body: Value = response.json().await.expect("Problem body");
        assert_eq!(
            body,
            json!({
                "type": "about:blank", "title": "Payload Too Large", "status": 413,
                "detail": "Maximum upload size exceeded", "instance": route(&a),
            })
        );
    }
    // Exactly the part limit is accepted.
    let mut at_limit = png(8, 8);
    at_limit.resize(part.len(), 0);
    uploaded(&server, &route(&a), file(at_limit)).await;
    server.stop().await;
}

#[tokio::test]
async fn a_failed_write_keeps_the_published_posters() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, a, _) = two_books(directory.path()).await;
    let before = list(&server, &a).await;
    let pool = db::main_pool(directory.path()).await;

    // The upload's row is inserted, then selecting it fails.
    db::inject_failure(&pool, "BEFORE UPDATE ON THUMBNAIL_BOOK").await;
    let response = server
        .admin(Method::POST, &route(&a))
        .multipart(file(png(8, 8)))
        .send()
        .await
        .expect("Upload request");
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    db::remove_failure(&pool).await;

    assert_eq!(list(&server, &a).await, before, "rolled back");
    let served = fetch(server.admin(Method::GET, &format!("/api/v1/books/{a}/thumbnail"))).await;
    assert!(served.body == cover(), "the sidecar is still served");
    server.stop().await;
}
