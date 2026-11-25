//! WebPub manifests of EPUB books (`WebPubGenerator.toManifestEpub`).

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::epub::{epub, sample};
use super::reading::access::create_reader;
use super::reading::fetch;
use super::{PNG, library_with_books};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::db::seed;

const WEBPUB: &str = "application/webpub+json";
const PAGE: &[u8] = b"<html><body><p>Page</p></body></html>";

/// A fixed-layout book whose navigation has nested entries with fragments.
fn navigation_book() -> Vec<u8> {
    let opf = r#"<package><metadata><meta property="rendition:layout">pre-paginated</meta></metadata>
      <manifest>
        <item id="nav" href="nav.xhtml" properties="nav" media-type="application/xhtml+xml"/>
        <item id="a" href="a.xhtml" media-type="application/xhtml+xml"/>
      </manifest><spine><itemref idref="a"/></spine></package>"#;
    let nav = r#"<html><body><nav epub:type="toc"><ol>
        <li><a href="a.xhtml">Part</a><ol><li><a href="a.xhtml#ch%201">One</a></li></ol></li>
      </ol></nav></body></html>"#;
    epub(&[
        ("OPS/book.opf", opf.as_bytes()),
        ("OPS/nav.xhtml", nav.as_bytes()),
        ("OPS/a.xhtml", PAGE),
    ])
}

async fn manifest(response: super::reading::Fetched) -> Value {
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.header(&CONTENT_TYPE), WEBPUB);
    serde_json::from_slice(&response.body).expect("JSON manifest")
}

/// `BOOK.LAST_MODIFIED_DATE`, stored in UTC.
async fn last_modified(pool: &SqlitePool, book_id: &str) -> Timestamp {
    let stored: String = sqlx::query_scalar("SELECT LAST_MODIFIED_DATE FROM BOOK WHERE ID = ?")
        .bind(book_id)
        .fetch_one(pool)
        .await
        .expect("Book date");
    let stored: DateTime = stored.parse().expect("Stored date");
    stored
        .to_zoned_datetime(TimeZone::UTC)
        .expect("In range")
        .timestamp()
}

/// The metadata of the sample book, from its package document.
fn assert_sample_metadata(metadata: &Value, last_modified: Timestamp) {
    let modified: Timestamp = metadata["modified"]
        .as_str()
        .and_then(|value| value.parse().ok())
        .expect("ISO date time");
    assert_eq!(modified, last_modified);
    let mut metadata = metadata.clone();
    metadata
        .as_object_mut()
        .expect("Metadata object")
        .remove("modified");
    assert_eq!(
        metadata,
        json!({
            "title": "The Incomplete Theft",
            "conformsTo": "https://readium.org/webpub-manifest/profiles/epub",
            "contributor": ["Ralph Burke"],
            "language": "en",
            "published": "2021-06-20",
            "numberOfPages": 14,
            "belongsTo": {"series": [{"name": "Series", "position": 1.0}]},
            "rendition": {"layout": "reflowable"},
        })
    );
}

#[tokio::test]
async fn epub_manifests_list_spine_pages_assets_and_navigation() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[(
            "Series/theft.epub",
            sample("The Incomplete Theft - Ralph Burke.epub"),
        )],
    )
    .await;
    let id = book(&pool, "theft").await.0;
    let origin = format!("http://127.0.0.1:{}", server.port());
    let api = format!("{origin}/api/v1/books/{id}");
    let get = |path: &str| fetch(server.admin(Method::GET, &format!("/api/v1/books/{id}/{path}")));

    let body = manifest(get("manifest").await).await;
    assert_eq!(body, manifest(get("manifest/epub").await).await);
    assert_eq!(
        body["context"],
        json!("https://readium.org/webpub-manifest/context.jsonld")
    );
    assert_sample_metadata(&body["metadata"], last_modified(&pool, &id).await);
    assert_eq!(
        body["links"],
        json!([
            {"rel": "self", "href": format!("{api}/manifest"), "type": WEBPUB},
            {"rel": "http://opds-spec.org/acquisition", "href": format!("{api}/file"), "type": "application/epub+zip"},
        ])
    );
    assert_eq!(body["images"], json!([]));

    // Spine order, not manifest order; `@` needs no escape in a path.
    let split = |part: u8| {
        json!({
            "href": format!("{api}/resource/OEBPS/@public@vhost@g@gutenberg@html@files@65659@65659-h@65659-h-0.htm_split_00{part}.html"),
            "type": "application/xhtml+xml",
        })
    };
    let titlepage = format!("{api}/resource/titlepage.xhtml");
    assert_eq!(
        body["readingOrder"],
        json!([{"href": titlepage, "type": "application/xhtml+xml"}, split(0), split(1), split(2)])
    );
    let resources = body["resources"].as_array().expect("Resources");
    assert_eq!(
        resources[0],
        json!({"href": format!("{api}/thumbnail"), "type": "image/jpeg"})
    );
    assert_eq!(
        resources.last(),
        Some(
            &json!({"href": format!("{api}/resource/nav.xhtml"), "type": "application/xhtml+xml"})
        )
    );
    assert_eq!(resources.len(), 1 + 4, "the thumbnail and the four assets");
    let toc = split(1)["href"].clone();
    assert_eq!(
        body["toc"],
        json!([{"title": "The Incomplete Theft", "href": toc}])
    );
    assert_eq!(
        body["landmarks"],
        json!([{"title": "Cover", "href": titlepage}])
    );
    assert_eq!(body["pageList"], json!([]));

    // Links follow the proxy in front of the server.
    let proxied = server
        .admin(Method::GET, &format!("/api/v1/books/{id}/manifest/epub"))
        .header("X-Forwarded-Proto", "https")
        .header("X-Forwarded-Host", "proxy.example")
        .header("X-Forwarded-Prefix", "/komga/");
    let proxied = manifest(fetch(proxied).await).await;
    assert_eq!(
        proxied["links"][0]["href"],
        json!(format!(
            "https://proxy.example/komga/api/v1/books/{id}/manifest"
        ))
    );
    server.stop().await;
}

#[tokio::test]
async fn epub_manifests_show_metadata_fixed_layouts_and_divina_links() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let png = cbz::fixture(PNG);
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/navigation.epub", navigation_book()),
            ("Series/divina.epub", super::epub::divina_book(&png)),
            ("Series/comic.cbz", cbz::zip(&[("p.png", STORED, &png)])),
        ],
    )
    .await;
    let (id, series_id) = book(&pool, "navigation").await;
    seed::book_metadata(
        &pool,
        &id,
        ("A summary", "9780000000002", "2020-02-03"),
        &[
            ("Ann", "author"),
            ("Pen", "penciller"),
            ("Bob", "author"),
            ("Pat", "penciler"),
            ("Wes", "writer"),
        ],
        &["b", "a", "b"],
    )
    .await;
    seed::series_reading(&pool, &series_id, "ja", "RIGHT_TO_LEFT").await;
    let get = |id: &str| fetch(server.admin(Method::GET, &format!("/api/v1/books/{id}/manifest")));

    let body = manifest(get(&id).await).await;
    let metadata = &body["metadata"];
    for (key, expected) in [
        ("description", json!("A summary")),
        ("identifier", json!("urn:isbn:9780000000002")),
        ("published", json!("2020-02-03")),
        ("author", json!(["Ann", "Bob"])),
        ("penciler", json!(["Pat", "Pen"])),
        ("contributor", json!(["Wes"])),
        ("subject", json!(["b", "a"])),
        ("language", json!("ja")),
        ("readingProgression", json!("rtl")),
        ("rendition", json!({"layout": "fixed"})),
    ] {
        assert_eq!(metadata[key], expected, "{key}");
    }
    let api = format!("http://127.0.0.1:{}/api/v1/books/{id}", server.port());
    assert_eq!(
        body["toc"],
        json!([{
            "title": "Part",
            "href": format!("{api}/resource/OPS/a.xhtml"),
            "children": [{"title": "One", "href": format!("{api}/resource/OPS/a.xhtml#ch 1")}],
        }])
    );

    let divina = book(&pool, "divina").await.0;
    let links = manifest(get(&divina).await).await["links"].clone();
    assert_eq!(
        links[1],
        json!({
            "href": format!("http://127.0.0.1:{}/api/v1/books/{divina}/manifest/divina", server.port()),
            "type": "application/divina+json",
        })
    );

    let comic = book(&pool, "comic").await.0;
    let path = format!("/api/v1/books/{comic}/manifest/epub");
    let other_profile = fetch(server.admin(Method::GET, &path)).await;
    assert_eq!(other_profile.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        other_profile.message(),
        "Book media type 'application/zip' not compatible with requested profile"
    );
    for path in ["/api/v1/books/0/manifest", "/api/v1/books/0/manifest/epub"] {
        let unknown = fetch(server.admin(Method::GET, path)).await;
        assert_eq!(unknown.status, StatusCode::NOT_FOUND, "{path}");
    }
    let path = format!("/api/v1/books/{id}/manifest");
    let anonymous = fetch(server.request(Method::GET, &path)).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    let outsider = create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    for path in [path.clone(), format!("{path}/epub")] {
        let denied = fetch(outsider.get(&server, &path)).await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN, "{path}");
    }
    server.stop().await;
}
