//! Kobo devices: URL credentials, the handshake, book metadata, covers and book files
//! (`KoboController`).

mod library;
mod proxy;

use komga_archive::epub::EpubPackage;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::books::epub::{divina_book, sample};
use crate::books::reading::access::{Reader, create_reader};
use crate::books::{PNG, library_with_books};
use crate::support::cbz;
use crate::support::db::content::book;
use crate::support::{TestServer, json};

pub(crate) async fn api_key(server: &TestServer, reader: Option<&Reader>) -> String {
    let path = "/api/v2/users/me/api-keys";
    let request = match reader {
        Some(reader) => reader.request(server, Method::POST, path),
        None => server.admin(Method::POST, path),
    };
    let response = request
        .json(&json!({"comment": "kobo"}))
        .send()
        .await
        .expect("API key creation");
    assert_eq!(response.status(), StatusCode::OK);
    json(response).await["key"]
        .as_str()
        .expect("Plaintext key")
        .to_owned()
}

async fn selected_thumbnail(pool: &SqlitePool, book_id: &str) -> String {
    sqlx::query_scalar("SELECT ID FROM THUMBNAIL_BOOK WHERE BOOK_ID = ? AND SELECTED")
        .bind(book_id)
        .fetch_one(pool)
        .await
        .expect("Selected thumbnail")
}

async fn status(server: &TestServer, method: Method, path: &str) -> StatusCode {
    server
        .request(method, path)
        .send()
        .await
        .expect("Kobo request")
        .status()
}

async fn update_settings(server: &TestServer, settings: Value) {
    let response = server
        .admin(Method::PATCH, "/api/v1/settings")
        .json(&settings)
        .send()
        .await
        .expect("Settings update");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn kobo_requests_need_a_key_in_the_url_and_the_kobo_role() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let admin_key = api_key(&server, None).await;
    let reader = create_reader(&server, "reader@example.org", json!({"roles": []})).await;
    let reader_key = api_key(&server, Some(&reader)).await;

    let ping = server
        .request(Method::GET, &format!("/kobo/{admin_key}/ping"))
        .send()
        .await
        .expect("Ping");
    assert_eq!(ping.status(), StatusCode::OK);
    assert_eq!(
        ping.headers()[CONTENT_TYPE],
        "application/json;charset=utf-8"
    );
    assert_eq!(ping.text().await.expect("Ping body"), "pong");
    // Spring's catch-all takes the methods a mapped path leaves out.
    let unmapped_method = server
        .request(Method::PUT, &format!("/kobo/{admin_key}/v1/library/sync"))
        .send()
        .await
        .expect("PUT sync");
    assert_eq!(unmapped_method.status(), StatusCode::OK);
    assert_eq!(json(unmapped_method).await, json!({}));

    let unknown = status(&server, Method::GET, "/kobo/unknown/ping").await;
    assert_eq!(unknown, StatusCode::UNAUTHORIZED);
    let header_only = server
        .request(Method::GET, "/kobo/unknown/ping")
        .header("X-API-Key", &admin_key)
        .send()
        .await
        .expect("Ping with a header key");
    assert_eq!(header_only.status(), StatusCode::UNAUTHORIZED);
    let session_only = server
        .admin(Method::GET, "/kobo/unknown/ping")
        .send()
        .await
        .expect("Ping with a session");
    assert_eq!(session_only.status(), StatusCode::UNAUTHORIZED);
    let without_role = status(&server, Method::GET, &format!("/kobo/{reader_key}/ping")).await;
    assert_eq!(without_role, StatusCode::FORBIDDEN);
    let api = status(
        &server,
        Method::GET,
        &format!("/api/v1/books?key={admin_key}"),
    )
    .await;
    assert_eq!(api, StatusCode::UNAUTHORIZED, "URL keys are for Kobo only");
    server.stop().await;
}

#[tokio::test]
async fn a_device_initializes_then_reads_metadata_and_covers() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let epub = divina_book(&cbz::fixture(PNG));
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Series/book.epub", epub)]).await;
    let (book_id, series_id) = book(&pool, "book").await;
    let key = api_key(&server, None).await;
    let origin = format!("http://127.0.0.1:{}", server.port());

    let initialization = server
        .request(Method::GET, &format!("/kobo/{key}/v1/initialization"))
        .header("X-Kobo-DeviceId", "device")
        .send()
        .await
        .expect("Initialization");
    assert_eq!(initialization.status(), StatusCode::OK);
    assert_eq!(initialization.headers()["x-kobo-apitoken"], "e30=");
    let resources = json(initialization).await["Resources"].take();
    assert_eq!(resources["image_host"], origin.as_str());
    let template = resources["image_url_template"]
        .as_str()
        .expect("Image URL template");
    assert_eq!(
        template,
        format!(
            "{origin}/kobo/{key}/v1/books/{{ImageId}}/thumbnail/{{Width}}/{{Height}}/false/image.jpg"
        )
    );
    assert_eq!(
        resources["image_url_quality_template"],
        format!(
            "{origin}/kobo/{key}/v1/books/{{ImageId}}/thumbnail/{{Width}}/{{Height}}/{{Quality}}/{{IsGreyscale}}/image.jpg"
        )
    );
    assert_eq!(
        resources["library_sync"],
        "https://storeapi.kobo.com/v1/library/sync"
    );

    let auth = server
        .request(Method::POST, &format!("/kobo/{key}/v1/auth/device"))
        .json(&json!({"UserKey": "user-key", "AffiliateName": "Kobo"}))
        .send()
        .await
        .expect("Device authentication");
    assert_eq!(auth.status(), StatusCode::OK);
    let auth = json(auth).await;
    assert_eq!(auth["UserKey"], "user-key");
    assert_eq!(auth["TokenType"], "Bearer");
    assert_eq!(auth["AccessToken"].as_str().map(str::len), Some(24));

    let metadata = server
        .request(
            Method::GET,
            &format!("/kobo/{key}/v1/library/{book_id}/metadata"),
        )
        .send()
        .await
        .expect("Metadata");
    assert_eq!(metadata.status(), StatusCode::OK);
    let metadata = json(metadata).await;
    let thumbnail_id = selected_thumbnail(&pool, &book_id).await;
    assert_book_metadata(
        &metadata,
        &origin,
        &key,
        &book_id,
        &series_id,
        &thumbnail_id,
    );

    let cover_path = template
        .strip_prefix(&origin)
        .expect("Local template")
        .replace("{ImageId}", &thumbnail_id)
        .replace("{Width}", "800")
        .replace("{Height}", "1200");
    let cover = server
        .request(Method::GET, &cover_path)
        .send()
        .await
        .expect("Cover");
    assert_eq!(cover.status(), StatusCode::OK);
    assert_eq!(cover.headers()[CONTENT_TYPE], "image/jpeg");
    let bytes = cover.bytes().await.expect("Cover bytes");
    assert!(bytes.starts_with(&[0xff, 0xd8]), "A JPEG");

    let elsewhere = server
        .request(Method::PUT, &format!("/kobo/{key}/v1/user/wishlist"))
        .send()
        .await
        .expect("Unhandled request");
    assert_eq!(elsewhere.status(), StatusCode::OK);
    assert_eq!(json(elsewhere).await, json!({}));
    server.stop().await;
}

fn assert_book_metadata(
    metadata: &Value,
    origin: &str,
    key: &str,
    book_id: &str,
    series_id: &str,
    thumbnail_id: &str,
) {
    let [book] = metadata.as_array().expect("Metadata list").as_slice() else {
        panic!("One book: {metadata}");
    };
    assert_eq!(book["EntitlementId"], book_id);
    assert_eq!(book["CoverImageId"], thumbnail_id);
    assert_eq!(book["Title"], "book");
    assert_eq!(book["Description"], " ");
    assert_eq!(book["Language"], "en");
    assert_eq!(book["Series"]["Id"], series_id);
    assert_eq!(book["Series"]["Name"], "Series");
    assert!(book.get("Isbn").is_none(), "Blank ISBN left out");
    let [download] = book["DownloadUrls"]
        .as_array()
        .expect("Downloads")
        .as_slice()
    else {
        panic!("One download: {book}");
    };
    assert_eq!(download["Format"], "EPUB3FL", "A fixed layout book");
    assert_eq!(
        download["Url"],
        format!("{origin}/kobo/{key}/v1/books/{book_id}/file/epub?convert_kepub=false")
    );
}

#[tokio::test]
async fn kobo_content_follows_the_device_owners_access() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let epub = divina_book(&cbz::fixture(PNG));
    let novel = sample("The Incomplete Theft - Ralph Burke.epub");
    let books = [("Series/book.epub", epub), ("Novel/novel.epub", novel)];
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let (book_id, _) = book(&pool, "book").await;
    let (novel_id, _) = book(&pool, "novel").await;
    let thumbnail_id = selected_thumbnail(&pool, &book_id).await;
    let access =
        json!({"roles": ["KOBO_SYNC"], "sharedLibraries": {"all": false, "libraryIds": []}});
    let reader = create_reader(&server, "reader@example.org", access).await;
    let key = api_key(&server, Some(&reader)).await;
    let admin_key = api_key(&server, None).await;

    let metadata = format!("/kobo/{key}/v1/library/{book_id}/metadata");
    assert_eq!(
        status(&server, Method::GET, &metadata).await,
        StatusCode::FORBIDDEN
    );
    let cover = format!("/kobo/{key}/v1/books/{thumbnail_id}/thumbnail/80/120/false/image.jpg");
    assert_eq!(
        status(&server, Method::GET, &cover).await,
        StatusCode::FORBIDDEN
    );
    let unknown = format!("/kobo/{key}/v1/library/unknown/metadata");
    assert_eq!(
        status(&server, Method::GET, &unknown).await,
        StatusCode::NOT_FOUND
    );
    let unknown = format!("/kobo/{admin_key}/v1/library/unknown/metadata");
    let response = server
        .request(Method::GET, &unknown)
        .send()
        .await
        .expect("Metadata");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await, json!([]));
    let novel = format!("/kobo/{admin_key}/v1/library/{novel_id}/metadata");
    let response = server
        .request(Method::GET, &novel)
        .send()
        .await
        .expect("Metadata");
    let download = &json(response).await[0]["DownloadUrls"][0];
    assert_eq!(
        download["Format"], "KEPUB",
        "A reflowable EPUB is converted"
    );
    let url = download["Url"].as_str().expect("Download URL");
    assert!(url.ends_with("/file/epub?convert_kepub=true"), "{url}");

    update_settings(&server, json!({"koboProxy": true})).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Client");
    let store_cover = client
        .request(
            Method::GET,
            format!(
                "http://127.0.0.1:{}/kobo/{admin_key}/v1/books/unknown/thumbnail/800/800/false/image.jpg",
                server.port()
            ),
        )
        .send()
        .await
        .expect("Store cover");
    assert_eq!(store_cover.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        store_cover.headers()[LOCATION],
        "https://cdn.kobo.com/book-images/unknown/800/800/false/image.jpg"
    );
    server.stop().await;
}

#[tokio::test]
async fn image_urls_use_the_kobo_port_unless_a_proxy_forwards_the_request() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let key = api_key(&server, None).await;
    let image_host = |headers: &'static [(&'static str, &'static str)]| {
        let mut request = server.request(Method::GET, &format!("/kobo/{key}/v1/initialization"));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        async move {
            let response = request.send().await.expect("Initialization");
            json(response).await["Resources"]["image_host"].take()
        }
    };

    let port = server.port();
    assert_eq!(
        image_host(&[("Host", "localhost")]).await,
        format!("http://localhost:{port}")
    );
    const FORWARDED: &[(&str, &str)] = &[
        ("X-Forwarded-Proto", "https"),
        ("X-Forwarded-Host", "demo.komga.org"),
    ];
    assert_eq!(image_host(FORWARDED).await, "https://demo.komga.org");
    update_settings(&server, json!({"koboPort": 8085})).await;
    assert_eq!(
        image_host(&[("Host", "localhost")]).await,
        "http://localhost:8085"
    );
    assert_eq!(image_host(FORWARDED).await, "https://demo.komga.org");
    server.stop().await;
}

async fn download(server: &TestServer, path: &str) -> reqwest::Response {
    server
        .request(Method::GET, path)
        .send()
        .await
        .expect("Book file")
}

#[tokio::test]
async fn kobo_downloads_kepub_forms_that_read_as_kobo_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let novel = sample("The Incomplete Theft - Ralph Burke.epub");
    let books = [("Novel/novel.epub", novel.clone())];
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let (novel_id, _) = book(&pool, "novel").await;
    let key = api_key(&server, None).await;
    let file = format!("/kobo/{key}/v1/books/{novel_id}/file/epub");

    let original = download(&server, &file).await;
    assert_eq!(original.status(), StatusCode::OK);
    assert!(original.bytes().await.expect("Original") == novel);

    let response = download(&server, &format!("{file}?convert_kepub=true")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/epub+zip");
    let disposition = response.headers()[CONTENT_DISPOSITION]
        .to_str()
        .expect("Disposition");
    assert!(disposition.contains("novel.kepub.epub"), "{disposition}");
    let length: usize = response.headers()[CONTENT_LENGTH]
        .to_str()
        .expect("Length")
        .parse()
        .expect("Length");
    let kepub = response.bytes().await.expect("KEPUB");
    assert_eq!(kepub.len(), length);
    assert_eq!(&kepub[30..58], b"mimetypeapplication/epub+zip");
    let path = directory.path().join("novel.kepub.epub");
    std::fs::write(&path, &kepub).expect("KEPUB file");
    let mut package = EpubPackage::open(&path).expect("A readable EPUB");
    let resources = package.resources().expect("Resources");
    assert!(package.is_kepub(&resources), "Kobo finds its spans");
    assert_eq!(
        resources.len(),
        EpubPackage::open(&directory.path().join("comics/Novel/novel.epub"))
            .expect("Original package")
            .resources()
            .expect("Original resources")
            .len(),
        "the KEPUB holds every resource of the book"
    );
    server.stop().await;
}

#[tokio::test]
async fn failed_kepub_deliveries_are_errors() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let novel = sample("The Incomplete Theft - Ralph Burke.epub");
    let books = [
        ("Novel/novel.epub", novel),
        ("Comics/comic.cbz", cbz::fixture(PNG)),
    ];
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let (novel_id, _) = book(&pool, "novel").await;
    let (comic_id, _) = book(&pool, "comic").await;
    let key = api_key(&server, None).await;
    let access = json!({"roles": ["KOBO_SYNC"]});
    let reader = create_reader(&server, "reader@example.org", access).await;
    let reader_key = api_key(&server, Some(&reader)).await;
    let kepub =
        |key: &str, id: &str| format!("/kobo/{key}/v1/books/{id}/file/epub?convert_kepub=true");

    assert_eq!(
        status(&server, Method::GET, &kepub(&reader_key, &novel_id)).await,
        StatusCode::FORBIDDEN,
        "downloads need the FILE_DOWNLOAD role"
    );
    assert_eq!(
        status(&server, Method::GET, &kepub(&key, "unknown")).await,
        StatusCode::NOT_FOUND
    );
    let comic = download(&server, &kepub(&key, &comic_id)).await;
    assert_eq!(comic.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json(comic).await["message"], "Kepub conversion failed");

    let path = directory.path().join("comics/Novel/novel.epub");
    std::fs::write(&path, b"not an EPUB any more").expect("Corrupt the book");
    let corrupt = download(&server, &kepub(&key, &novel_id)).await;
    assert_eq!(corrupt.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json(corrupt).await["message"], "Kepub conversion failed");
    std::fs::remove_file(&path).expect("Remove the book");
    assert_eq!(
        status(&server, Method::GET, &kepub(&key, &novel_id)).await,
        StatusCode::NOT_FOUND
    );
    server.stop().await;
}
