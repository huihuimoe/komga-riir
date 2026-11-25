//! DiViNa and PDF WebPub manifests (`WebPubGenerator.toManifestDivina`/`toManifestPdf`).

use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::epub::divina_book;
use super::reading::access::create_reader;
use super::reading::fetch;
use super::{JPEG, PNG, library_with_books};
use crate::support::TestServer;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;

const DIVINA: &str = "application/divina+json";
const WEBPUB: &str = "application/webpub+json";
const SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#;

async fn manifest(server: &TestServer, path: &str, media_type: &str) -> Value {
    let response = fetch(server.admin(Method::GET, path)).await;
    assert_eq!(response.status, StatusCode::OK, "{path}");
    assert_eq!(response.header(&CONTENT_TYPE), media_type, "{path}");
    serde_json::from_slice(&response.body).expect("JSON manifest")
}

/// `(width, height)` of each page the page list shows.
async fn page_sizes(server: &TestServer, id: &str) -> Vec<(Value, Value)> {
    let response = fetch(server.admin(Method::GET, &format!("/api/v1/books/{id}/pages"))).await;
    let pages: Value = serde_json::from_slice(&response.body).expect("JSON pages");
    pages
        .as_array()
        .expect("Pages")
        .iter()
        .map(|page| (page["width"].clone(), page["height"].clone()))
        .collect()
}

#[tokio::test]
async fn divina_manifests_list_pages_with_jpeg_alternates() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let webp = cbz::fixture("komga-image/tests/fixtures/lossless.webp");
    let comic = cbz::zip(&[
        ("p1.jpg", STORED, &cbz::fixture(JPEG)),
        ("p2.webp", STORED, &webp),
        ("p3.svg", STORED, SVG),
    ]);
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/comic.cbz", comic),
            ("Series/divina.epub", divina_book(&cbz::fixture(PNG))),
        ],
    )
    .await;
    let id = book(&pool, "comic").await.0;
    let api = format!("http://127.0.0.1:{}/api/v1/books/{id}", server.port());

    let body = manifest(&server, &format!("/api/v1/books/{id}/manifest"), DIVINA).await;
    let divina_path = format!("/api/v1/books/{id}/manifest/divina");
    assert_eq!(body, manifest(&server, &divina_path, DIVINA).await);
    assert_eq!(
        body["metadata"]["conformsTo"],
        json!("https://readium.org/webpub-manifest/profiles/divina")
    );
    assert_eq!(body["metadata"].get("rendition"), None);
    assert_eq!(
        body["links"],
        json!([
            {"rel": "self", "href": format!("{api}/manifest"), "type": DIVINA},
            {"rel": "http://opds-spec.org/acquisition", "href": format!("{api}/file"), "type": "application/vnd.comicbook+zip"},
        ])
    );
    let sizes = page_sizes(&server, &id).await;
    assert!(
        sizes[..2].iter().all(|(width, _)| width.is_u64()),
        "{sizes:?}"
    );
    let page = |number: usize, media_type: &str| {
        let (width, height) = sizes[number - 1].clone();
        let href = format!("{api}/pages/{number}?contentNegotiation=false");
        json!({"href": href, "type": media_type, "width": width, "height": height})
    };
    let mut webp_page = page(2, "image/webp");
    let alternate = json!([{
        "href": format!("{api}/pages/2?contentNegotiation=false&convert=jpeg"),
        "type": "image/jpeg",
        "width": sizes[1].0,
        "height": sizes[1].1,
    }]);
    webp_page["alternate"] = alternate;
    assert_eq!(
        body["readingOrder"],
        json!([page(1, "image/jpeg"), webp_page, {
            "href": format!("{api}/pages/3?contentNegotiation=false"),
            "type": "image/svg+xml",
        }])
    );
    assert_eq!(
        body["resources"],
        json!([{"href": format!("{api}/thumbnail"), "type": "image/jpeg"}])
    );
    for path in ["manifest/pdf", "manifest/epub"] {
        let path = format!("/api/v1/books/{id}/{path}");
        let other_profile = fetch(server.admin(Method::GET, &path)).await;
        assert_eq!(other_profile.status, StatusCode::BAD_REQUEST, "{path}");
    }

    // DiViNa EPUB books read their image pages.
    let epub = book(&pool, "divina").await.0;
    let path = format!("/api/v1/books/{epub}/manifest/divina");
    let reading_order = manifest(&server, &path, DIVINA).await["readingOrder"].clone();
    assert_eq!(reading_order.as_array().map(Vec::len), Some(2));
    assert_eq!(reading_order[0]["type"], json!("image/png"));

    let outsider = create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    let denied = fetch(outsider.get(&server, &divina_path)).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    server.stop().await;
}

#[tokio::test]
async fn pdf_manifests_list_raw_pages_and_divina_manifests_rendered_ones() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let pdf = cbz::fixture("komga-archive/tests/fixtures/pdf/pages.pdf");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/pages.pdf", pdf)]).await;
    let id = book(&pool, "pages").await.0;
    let api = format!("http://127.0.0.1:{}/api/v1/books/{id}", server.port());

    let body = manifest(&server, &format!("/api/v1/books/{id}/manifest"), WEBPUB).await;
    assert_eq!(
        body,
        manifest(&server, &format!("/api/v1/books/{id}/manifest/pdf"), WEBPUB).await
    );
    assert_eq!(
        body["metadata"]["conformsTo"],
        json!("https://readium.org/webpub-manifest/profiles/pdf")
    );
    assert_eq!(
        body["links"],
        json!([
            {"rel": "self", "href": format!("{api}/manifest"), "type": WEBPUB},
            {"href": format!("{api}/manifest/divina"), "type": DIVINA},
            {"rel": "http://opds-spec.org/acquisition", "href": format!("{api}/file"), "type": "application/pdf"},
        ])
    );
    let sizes = page_sizes(&server, &id).await;
    assert!(
        sizes
            .iter()
            .all(|(width, height)| width.is_u64() && height.is_u64()),
        "{sizes:?}"
    );
    let raw: Vec<Value> = (1..=sizes.len())
        .map(|number| json!({"href": format!("{api}/pages/{number}/raw"), "type": "application/pdf"}))
        .collect();
    assert_eq!(body["readingOrder"], json!(raw));
    assert_eq!(body["metadata"]["numberOfPages"], json!(sizes.len()));

    // Pages as rendered, which need no alternate.
    let path = format!("/api/v1/books/{id}/manifest/divina");
    let divina = manifest(&server, &path, DIVINA).await;
    let rendered: Vec<Value> = sizes
        .iter()
        .enumerate()
        .map(|(index, (width, height))| {
            json!({
                "href": format!("{api}/pages/{}?contentNegotiation=false", index + 1),
                "type": "image/jpeg",
                "width": width,
                "height": height,
            })
        })
        .collect();
    assert_eq!(divina["readingOrder"], json!(rendered));
    server.stop().await;
}
