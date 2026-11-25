//! Series posters: listing, reading, uploading, selecting and deleting.

use std::fs;
use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::books::reading::access::create_reader;
use crate::books::reading::fetch;
use crate::books::thumbnails::{book_with_first_page, cover};
use crate::support::db::{self, content::book};
use crate::support::posters::{file, list_at, png, selected_ids, status_and_body};
use crate::support::tasks::wait_for_scan_work;
use crate::support::{TestServer, openapi};

fn route(series_id: &str) -> String {
    format!("/api/v1/series/{series_id}/thumbnails")
}

/// Series `S` and `T` with a `cover.png` sidecar each, and the oneshot `one`; the IDs of `S`, `T`
/// and the oneshot series.
async fn library(directory: &Path) -> (TestServer, SqlitePool, [String; 3]) {
    let server = TestServer::start_claimed(directory).await;
    let root = directory.join("comics");
    for path in ["S", "T", "_oneshots"] {
        fs::create_dir_all(root.join(path)).expect("Series directory");
    }
    for (path, content) in [
        ("S/s.cbz", book_with_first_page(&cover())),
        ("S/cover.png", cover()),
        ("T/t.cbz", book_with_first_page(&cover())),
        ("T/cover.png", png(30, 20)),
        ("_oneshots/one.cbz", book_with_first_page(&cover())),
    ] {
        fs::write(root.join(path), content).expect("Library file");
    }
    let response = server
        .admin(Method::POST, "/api/v1/libraries")
        .json(&json!({"name": "Comics", "root": root, "oneshotsDirectory": "_oneshots"}))
        .send()
        .await
        .expect("Library creation");
    assert_eq!(response.status(), StatusCode::OK);
    wait_for_scan_work(directory).await;
    let pool = db::main_pool(directory).await;
    let mut ids = Vec::new();
    for name in ["s", "t", "one"] {
        ids.push(book(&pool, name).await.1);
    }
    (server, pool, ids.try_into().expect("Three series"))
}

async fn list(server: &TestServer, series_id: &str) -> Vec<Value> {
    list_at(server, &route(series_id)).await
}

async fn upload(server: &TestServer, series_id: &str, query: &str) -> Value {
    let request = server.admin(Method::POST, &format!("{}{query}", route(series_id)));
    let response = request
        .multipart(file(png(120, 80)))
        .send()
        .await
        .expect("Upload");
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::POST, response).await
}

fn id(poster: &Value) -> &str {
    poster["id"].as_str().expect("Poster ID")
}

#[tokio::test]
async fn series_posters_are_uploaded_selected_and_deleted() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, [s, _, _]) = library(directory.path()).await;
    let posters = list(&server, &s).await;
    let sidecar = posters[0].clone();
    assert_eq!(
        posters,
        [json!({
            "id": sidecar["id"], "seriesId": s, "type": "SIDECAR", "selected": true,
            "mediaType": "image/png", "fileSize": cover().len(), "width": 800, "height": 400,
        })]
    );
    let by_id = fetch(server.admin(Method::GET, &format!("{}/{}", route(&s), id(&sidecar))));
    assert!(by_id.await.body == cover(), "the sidecar bytes");

    let first = upload(&server, &s, "").await;
    assert_eq!(
        (&first["type"], &first["selected"]),
        (&json!("USER_UPLOADED"), &json!(true))
    );
    let second = upload(&server, &s, "?selected=false").await;
    assert_eq!(second["selected"], false);
    assert_eq!(selected_ids(&list(&server, &s).await), [id(&first)]);
    let served = fetch(server.admin(Method::GET, &format!("/api/v1/series/{s}/thumbnail")));
    assert!(
        served.await.body == png(120, 80),
        "the selected upload is the series thumbnail"
    );

    let admin = |method, poster: &Value, suffix| {
        server.admin(method, &format!("{}/{}{suffix}", route(&s), id(poster)))
    };
    let (code, body) = status_and_body(admin(Method::DELETE, &sidecar, "")).await;
    assert_eq!(
        (code, &body["message"]),
        (
            StatusCode::BAD_REQUEST,
            &json!("Only uploaded thumbnails can be deleted")
        )
    );
    assert_eq!(
        status_and_body(admin(Method::DELETE, &second, "")).await.0,
        StatusCode::ACCEPTED
    );

    // Deleting the selected upload selects nothing until the series thumbnail is read.
    assert_eq!(
        status_and_body(admin(Method::DELETE, &first, "")).await.0,
        StatusCode::ACCEPTED
    );
    assert!(selected_ids(&list(&server, &s).await).is_empty());
    let served = fetch(server.admin(Method::GET, &format!("/api/v1/series/{s}/thumbnail")));
    assert!(served.await.body == cover(), "the sidecar again");
    assert_eq!(selected_ids(&list(&server, &s).await), [id(&sidecar)]);
    server.stop().await;
}

#[tokio::test]
async fn invalid_series_poster_requests_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool, [s, t, oneshot]) = library(directory.path()).await;
    let other = list(&server, &t).await[0].clone();
    create_reader(&server, "reader@example.com", json!({"roles": []})).await;
    let none = json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}});
    create_reader(&server, "outsider@example.com", none).await;
    let as_user = |email: &str, method, path: &str| {
        server
            .request(method, path)
            .basic_auth(email, Some("secret"))
    };
    let poster = format!("{}/{}", route(&t), id(&other));
    let cases = [
        (
            server
                .admin(Method::POST, &route(&oneshot))
                .multipart(file(png(8, 8))),
            StatusCode::BAD_REQUEST,
        ),
        (
            server
                .admin(Method::POST, &route("missing"))
                .multipart(file(png(8, 8))),
            StatusCode::NOT_FOUND,
        ),
        (
            server
                .admin(Method::POST, &route(&s))
                .multipart(file(b"text".to_vec())),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            server.admin(
                Method::PUT,
                &format!("{}/{}/selected", route(&s), id(&other)),
            ),
            StatusCode::BAD_REQUEST,
        ),
        (
            server.admin(Method::PUT, &format!("{}/missing/selected", route(&s))),
            StatusCode::NOT_FOUND,
        ),
        (
            as_user("reader@example.com", Method::POST, &route(&s)).multipart(file(png(8, 8))),
            StatusCode::FORBIDDEN,
        ),
        (
            as_user("reader@example.com", Method::DELETE, &poster),
            StatusCode::FORBIDDEN,
        ),
        (
            as_user("reader@example.com", Method::GET, &poster),
            StatusCode::OK,
        ),
        (
            as_user("outsider@example.com", Method::GET, &route(&t)),
            StatusCode::FORBIDDEN,
        ),
        (
            as_user("outsider@example.com", Method::GET, &poster),
            StatusCode::FORBIDDEN,
        ),
        (
            as_user("outsider@example.com", Method::GET, &route("missing")),
            StatusCode::NOT_FOUND,
        ),
        (
            server.request(Method::GET, &route(&s)),
            StatusCode::UNAUTHORIZED,
        ),
    ];
    for (request, expected) in cases {
        let (code, body) = status_and_body(request).await;
        assert_eq!(code, expected, "{body}");
    }
    assert!(
        list(&server, "missing").await.is_empty(),
        "unlimited users skip the lookup"
    );

    // A failed selection rolls the upload back.
    let before = list(&server, &s).await;
    db::inject_failure(&pool, "BEFORE UPDATE ON THUMBNAIL_SERIES").await;
    let failed = server
        .admin(Method::POST, &route(&s))
        .multipart(file(png(8, 8)));
    assert_eq!(
        status_and_body(failed).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    db::remove_failure(&pool).await;
    assert_eq!(list(&server, &s).await, before);
    server.stop().await;
}
