//! OPDS 2 authentication and the book resources OPDS 2 publications link to.

use reqwest::header::{ACCEPT, CONTENT_TYPE, LINK, WWW_AUTHENTICATE};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::books::library_with_books;
use crate::books::reading::{fetch, pages_book};
use crate::support::db::content::book;

const AUTHENTICATION: &str = "application/opds-authentication+json";

#[tokio::test]
async fn clients_without_credentials_get_the_authentication_document() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("S/a.cbz", pages_book())]).await;
    let (id, _) = book(&pool, "a").await;
    let base = format!("http://127.0.0.1:{}", server.port());
    let document = json!({
        "authentication": [{
            "type": "http://opds-spec.org/auth/basic",
            "labels": {"login": "Email", "password": "Password"},
        }],
        "title": "Komga",
        "id": format!("{base}/opds/v2/auth"),
        "description": "Enter your email and password to authenticate.",
        "links": [
            {"rel": "help", "href": "https://komga.org"},
            {"rel": "logo", "href": format!("{base}/android-chrome-512x512.png")},
        ],
    });

    let anonymous = fetch(server.request(Method::GET, "/opds/v2/auth")).await;
    assert_eq!(anonymous.status, StatusCode::OK);
    assert_eq!(anonymous.header(&CONTENT_TYPE), AUTHENTICATION);
    assert_eq!(
        serde_json::from_slice::<Value>(&anonymous.body).expect("JSON"),
        document
    );

    let manifest = format!("/opds/v2/books/{id}/manifest");
    let wrong = server
        .request(Method::GET, &manifest)
        .basic_auth("admin@example.com", Some("wrong"));
    for request in [server.request(Method::GET, &manifest), wrong] {
        let denied = fetch(request).await;
        assert_eq!(denied.status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            denied.header(&CONTENT_TYPE),
            "application/opds-authentication+json;charset=UTF-8"
        );
        assert_eq!(denied.header(&WWW_AUTHENTICATE), r#"Basic realm="Realm""#);
        assert_eq!(
            denied.header(&LINK),
            format!(
                r#"<{base}/opds/v2/auth>; rel="http://opds-spec.org/auth/document"; type="{AUTHENTICATION}""#
            )
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&denied.body).expect("JSON"),
            document
        );
    }
    // Elsewhere Spring Security's HTTP Basic entry points come first, the document is the default.
    let manifest = format!("/api/v1/books/{id}/manifest");
    let api = |accept: &'static str| {
        fetch(
            server
                .request(Method::GET, &manifest)
                .header(ACCEPT, accept),
        )
    };
    for accept in [
        "*/*",
        "application/json",
        "application/*",
        "multipart/form-data",
    ] {
        let challenged = api(accept).await;
        assert_eq!(challenged.status, StatusCode::UNAUTHORIZED, "{accept}");
        assert_eq!(
            challenged.header(&WWW_AUTHENTICATE),
            r#"Basic realm="Realm""#,
            "{accept}"
        );
        assert!(challenged.headers.get(LINK).is_none(), "{accept}");
    }
    for accept in [
        "text/html",
        "text/plain",
        "application/json, */*;q=0.1",
        "bogus",
    ] {
        let document = api(accept).await;
        assert_eq!(
            document.header(&CONTENT_TYPE),
            "application/opds-authentication+json;charset=UTF-8",
            "{accept}"
        );
        assert!(document.headers.get(LINK).is_some(), "{accept}");
    }
    let script = fetch(
        server
            .request(Method::GET, &manifest)
            .header("X-Requested-With", "XMLHttpRequest"),
    )
    .await;
    assert_eq!(script.status, StatusCode::UNAUTHORIZED);
    assert!(script.headers.get(WWW_AUTHENTICATE).is_none());
    assert!(script.body.is_empty());
    server.stop().await;
}

#[tokio::test]
async fn publications_link_to_opds_resources() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("S/a.cbz", pages_book())]).await;
    let (id, series) = book(&pool, "a").await;
    let base = format!("http://127.0.0.1:{}", server.port());

    let response = fetch(server.admin(Method::GET, &format!("/opds/v2/books/{id}/manifest"))).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.header(&CONTENT_TYPE),
        "application/opds-publication+json"
    );
    let manifest: Value = serde_json::from_slice(&response.body).expect("JSON manifest");
    let authenticate = json!({"authenticate": {
        "href": format!("{base}/opds/v2/auth"),
        "type": AUTHENTICATION,
    }});
    let books = format!("{base}/opds/v2/books/{id}");
    assert_eq!(
        manifest["links"],
        json!([
            {"rel": "self", "href": format!("{books}/manifest"), "type": "application/divina+json", "properties": authenticate},
            {"rel": "http://opds-spec.org/acquisition", "href": format!("{books}/file"), "type": "application/vnd.comicbook+zip", "properties": authenticate},
            {"rel": "http://www.cantook.com/api/progression", "href": format!("{books}/progression"), "type": "application/vnd.readium.progression+json", "properties": authenticate},
        ])
    );
    assert_eq!(
        manifest["resources"],
        json!([{"href": format!("{books}/thumbnail"), "type": "image/jpeg", "properties": authenticate}])
    );
    assert_eq!(
        manifest["metadata"]["belongsTo"]["series"][0]["links"],
        json!([{"href": format!("{base}/opds/v2/series/{series}"), "type": "application/opds+json"}])
    );

    // Every reading order page and its raw form are served, counted from 1.
    for link in manifest["readingOrder"].as_array().expect("Reading order") {
        let href = link["href"].as_str().expect("href");
        let path = href.strip_prefix(&base).expect("A server link");
        assert!(
            path.starts_with(&format!("/opds/v2/books/{id}/pages/")),
            "{path}"
        );
        let page = fetch(server.admin(Method::GET, path)).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert_eq!(
            page.header(&CONTENT_TYPE),
            link["type"].as_str().expect("type")
        );
    }
    // Raw pages are PDF pages; the route is the REST one.
    for prefix in ["/api/v1", "/opds/v2"] {
        let raw =
            fetch(server.admin(Method::GET, &format!("{prefix}/books/{id}/pages/1/raw"))).await;
        assert_eq!(raw.status, StatusCode::BAD_REQUEST, "{prefix}");
    }
    server.stop().await;
}
