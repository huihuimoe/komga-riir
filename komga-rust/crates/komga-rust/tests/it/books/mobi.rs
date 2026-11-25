//! End-to-end `.mobi` format support (spec §16.4): scanning, analysis, cover thumbnails,
//! EXTH metadata, WebPub manifest & resources, raw download, Kobo EPUB/KEPUB conversion,
//! KOReader sync, and DRM/corrupt failure handling.

use std::fs;
use std::path::Path;

use komga_archive::epub::EpubPackage;
use komga_archive::mobi::MOBI_MEDIA_TYPE;
use reqwest::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::{JPEG, PNG, library_with_books};
use crate::kobo::api_key;
use crate::support::cbz;
use crate::support::db::content::{book, media};
use crate::support::tasks::{TaskRow, wait_for_tasks};
use crate::support::{TestServer, json};

fn sample_exth() -> Vec<u8> {
    let entries: Vec<(u32, &[u8])> = vec![
        (503, b"Mobi Odyssey"),
        (100, b"Jane Writer"),
        (101, b"Kindle Press"),
        (103, b"<p>Epic <i>story</i>.</p>"),
        (104, b"9780306406157"),
        (105, b"Adventure"),
        (106, b"2023-07-19"),
        (524, b"en"),
        (201, &[0, 0, 0, 0]),
    ];
    let mut exth = Vec::new();
    exth.extend_from_slice(b"EXTH");
    exth.extend_from_slice(&0_u32.to_be_bytes());
    exth.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (rec_type, payload) in entries {
        exth.extend_from_slice(&rec_type.to_be_bytes());
        exth.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
        exth.extend_from_slice(payload);
    }
    while exth.len() % 4 != 0 {
        exth.push(0);
    }
    let len = exth.len() as u32;
    exth[4..8].copy_from_slice(&len.to_be_bytes());
    exth
}

/// Builds a minimal valid uncompressed MOBI6 book with EXTH metadata, cover, inline image, and
/// two chapters separated by `<mbp:pagebreak/>`.
fn sample_mobi(cover_jpeg: &[u8], inline_png: &[u8], encrypted: bool) -> Vec<u8> {
    let html = concat!(
        "<html><body>",
        "<p>Opening paragraph with enough words for KEPUB spans. Second sentence here.</p>",
        "<mbp:pagebreak/>",
        "<p>Illustration chapter: <img recindex=\"00002\"/></p>",
        "</body></html>",
    );
    let mut r0 = vec![0_u8; 16 + 232];
    r0[0..2].copy_from_slice(&1_u16.to_be_bytes());
    r0[4..8].copy_from_slice(&(html.len() as u32).to_be_bytes());
    r0[8..10].copy_from_slice(&1_u16.to_be_bytes());
    r0[10..12].copy_from_slice(&4096_u16.to_be_bytes());
    r0[12..14].copy_from_slice(&(if encrypted { 2_u16 } else { 0_u16 }).to_be_bytes());

    let m = 16;
    r0[m..m + 4].copy_from_slice(b"MOBI");
    r0[m + 4..m + 8].copy_from_slice(&232_u32.to_be_bytes());
    r0[m + 8..m + 12].copy_from_slice(&2_u32.to_be_bytes());
    r0[m + 12..m + 16].copy_from_slice(&65001_u32.to_be_bytes());
    r0[m + 20..m + 24].copy_from_slice(&6_u32.to_be_bytes());
    r0[m + 92..m + 96].copy_from_slice(&2_u32.to_be_bytes());
    r0[m + 112..m + 116].copy_from_slice(&0x40_u32.to_be_bytes());
    r0[m + 152..m + 156].copy_from_slice(&u32::MAX.to_be_bytes());
    r0[m + 228..m + 232].copy_from_slice(&u32::MAX.to_be_bytes());
    r0.extend_from_slice(&sample_exth());
    let name_off = r0.len() as u32;
    r0.extend_from_slice(b"Mobi Odyssey");
    r0[m + 68..m + 72].copy_from_slice(&name_off.to_be_bytes());
    r0[m + 72..m + 76].copy_from_slice(&12_u32.to_be_bytes());

    let records = [
        r0,
        html.as_bytes().to_vec(),
        cover_jpeg.to_vec(),
        inline_png.to_vec(),
    ];
    let mut pdb = vec![0_u8; 78 + records.len() * 8];
    pdb[..4].copy_from_slice(b"Mobi");
    pdb[60..68].copy_from_slice(b"BOOKMOBI");
    pdb[76..78].copy_from_slice(&(records.len() as u16).to_be_bytes());
    let mut offset = pdb.len();
    for (i, rec) in records.iter().enumerate() {
        pdb[78 + i * 8..82 + i * 8].copy_from_slice(&(offset as u32).to_be_bytes());
        offset += rec.len();
    }
    for rec in records {
        pdb.extend_from_slice(&rec);
    }
    pdb
}

fn broken_mobi() -> Vec<u8> {
    let mut bytes = vec![0_u8; 80];
    bytes[60..68].copy_from_slice(b"BOOKMOBI");
    bytes
}

async fn get_json(server: &TestServer, path: &str) -> Value {
    let response = server
        .admin(Method::GET, path)
        .send()
        .await
        .expect("GET JSON");
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    json(response).await
}

async fn assert_metadata_and_media(server: &TestServer, pool: &SqlitePool, good_id: &str) {
    let (status, mt, comment, pages) = media(pool, good_id).await;
    assert_eq!(
        (status.as_str(), mt.as_deref(), comment.as_deref()),
        ("READY", Some(MOBI_MEDIA_TYPE), None)
    );
    assert!(pages >= 3);

    let book_dto = get_json(server, &format!("/api/v1/books/{good_id}")).await;
    assert_eq!(book_dto["media"]["mediaProfile"], "EPUB");
    assert_eq!(book_dto["media"]["mediaType"], MOBI_MEDIA_TYPE);
    assert_eq!(book_dto["metadata"]["title"], "Mobi Odyssey");
    assert_eq!(book_dto["metadata"]["summary"], "Epic story.");
    assert_eq!(book_dto["metadata"]["isbn"], "9780306406157");
    assert_eq!(book_dto["metadata"]["releaseDate"], "2023-07-19");

    let series_id = book_dto["seriesId"].as_str().expect("seriesId");
    let series_dto = get_json(server, &format!("/api/v1/series/{series_id}")).await;
    assert_eq!(series_dto["metadata"]["publisher"], "Kindle Press");
    assert_eq!(series_dto["metadata"]["language"], "en");
    assert_eq!(series_dto["metadata"]["genres"], json!(["adventure"]));
}

async fn assert_webpub_and_download(server: &TestServer, id: &str, good: &[u8], inline: &[u8]) {
    let thumb = server
        .admin(Method::GET, &format!("/api/v1/books/{id}/thumbnail"))
        .send()
        .await
        .expect("thumbnail");
    assert_eq!(thumb.status(), StatusCode::OK);
    assert!(
        thumb
            .bytes()
            .await
            .expect("thumb")
            .starts_with(&[0xff, 0xd8, 0xff])
    );

    let manifest = get_json(server, &format!("/api/v1/books/{id}/manifest")).await;
    assert_eq!(manifest["readingOrder"].as_array().expect("order").len(), 3);
    let positions = get_json(server, &format!("/api/v1/books/{id}/positions")).await;
    assert!(positions["total"].as_u64().expect("total") >= 3);

    let ch1 = server
        .admin(
            Method::GET,
            &format!("/api/v1/books/{id}/resource/OEBPS/text/chapter-0001.xhtml"),
        )
        .send()
        .await
        .expect("chapter 1");
    assert_eq!(ch1.status(), StatusCode::OK);
    assert!(
        ch1.text()
            .await
            .expect("ch1")
            .contains(r#"src="../images/asset-0002.png""#)
    );

    let img = server
        .admin(
            Method::GET,
            &format!("/api/v1/books/{id}/resource/OEBPS/images/asset-0002.png"),
        )
        .send()
        .await
        .expect("inline image");
    assert_eq!(img.status(), StatusCode::OK);
    assert_eq!(img.bytes().await.expect("png").as_ref(), inline);

    let raw = server
        .admin(Method::GET, &format!("/api/v1/books/{id}/file"))
        .send()
        .await
        .expect("raw download");
    assert_eq!(raw.status(), StatusCode::OK);
    assert_eq!(raw.headers()[CONTENT_TYPE], MOBI_MEDIA_TYPE);
    assert_eq!(raw.bytes().await.expect("raw").as_ref(), good);
}

async fn assert_kobo_downloads(server: &TestServer, key: &str, book_id: &str, dir: &Path) {
    let epub_resp = server
        .request(
            Method::GET,
            &format!("/kobo/{key}/v1/books/{book_id}/file/epub?convert_kepub=false"),
        )
        .send()
        .await
        .expect("Kobo EPUB download");
    assert_eq!(epub_resp.status(), StatusCode::OK);
    assert_eq!(epub_resp.headers()[CONTENT_TYPE], "application/epub+zip");
    assert!(
        epub_resp.headers()[CONTENT_DISPOSITION]
            .to_str()
            .expect("cd")
            .contains("odyssey.epub")
    );
    let epub_path = dir.join("kobo.epub");
    fs::write(&epub_path, epub_resp.bytes().await.expect("epub")).expect("write epub");
    let mut epub = EpubPackage::open(&epub_path).expect("valid EPUB");
    assert_eq!(epub.metadata().title.as_deref(), Some("Mobi Odyssey"));
    assert!(epub.cover().expect("cover").is_some());

    let kepub_resp = server
        .request(
            Method::GET,
            &format!("/kobo/{key}/v1/books/{book_id}/file/epub?convert_kepub=true"),
        )
        .send()
        .await
        .expect("Kobo KEPUB download");
    assert_eq!(kepub_resp.status(), StatusCode::OK);
    assert_eq!(kepub_resp.headers()[CONTENT_TYPE], "application/epub+zip");
    assert!(
        kepub_resp.headers()[CONTENT_DISPOSITION]
            .to_str()
            .expect("cd")
            .contains("odyssey.kepub.epub")
    );
    let kepub_path = dir.join("kobo.kepub.epub");
    fs::write(&kepub_path, kepub_resp.bytes().await.expect("kepub")).expect("write kepub");
    let mut kepub = EpubPackage::open(&kepub_path).expect("valid KEPUB");
    let resources = kepub.resources().expect("kepub resources");
    assert!(kepub.is_kepub(&resources));
}

async fn assert_koreader_sync(
    server: &TestServer,
    pool: &SqlitePool,
    library_id: &str,
    key: &str,
    config_dir: &Path,
) {
    let patch = server
        .admin(Method::PATCH, &format!("/api/v1/libraries/{library_id}"))
        .json(&json!({"hashKoreader": true}))
        .send()
        .await
        .expect("enable hashKoreader");
    assert_eq!(patch.status(), StatusCode::NO_CONTENT);
    wait_for_tasks(config_dir, <[TaskRow]>::is_empty).await;

    let hash: String =
        sqlx::query_scalar("SELECT FILE_HASH_KOREADER FROM BOOK WHERE NAME = 'odyssey'")
            .fetch_one(pool)
            .await
            .expect("koreader hash");
    assert_eq!(hash.len(), 32);

    let put = server
        .request(Method::PUT, "/koreader/syncs/progress")
        .header("X-Auth-User", key)
        .json(&json!({
            "document": hash,
            "progress": "/body/DocFragment[2].0",
            "percentage": 0.5,
            "device": "Kobo",
            "device_id": "dev-1"
        }))
        .send()
        .await
        .expect("PUT koreader");
    assert_eq!(put.status(), StatusCode::OK);

    let get = server
        .request(Method::GET, &format!("/koreader/syncs/progress/{hash}"))
        .header("X-Auth-User", key)
        .send()
        .await
        .expect("GET koreader");
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(json(get).await["progress"], "/body/DocFragment[2].0");
}

#[tokio::test]
async fn mobi_books_scan_analyze_serve_webpub_kobo_and_koreader() {
    let dir = komga_test_support::tempdir().expect("Data directory");
    let cover = cbz::fixture(JPEG);
    let inline = cbz::fixture(PNG);
    let good = sample_mobi(&cover, &inline, false);
    let drm = sample_mobi(&cover, &inline, true);

    let (server, library, pool) = library_with_books(
        dir.path(),
        &[
            ("Odyssey/odyssey.mobi", good.clone()),
            ("Broken/drm.mobi", drm),
            ("Broken/corrupt.mobi", broken_mobi()),
        ],
    )
    .await;
    wait_for_tasks(dir.path(), <[TaskRow]>::is_empty).await;

    let (good_id, _) = book(&pool, "odyssey").await;
    let (drm_id, _) = book(&pool, "drm").await;
    let (corrupt_id, _) = book(&pool, "corrupt").await;

    assert_metadata_and_media(&server, &pool, &good_id).await;
    assert_eq!(
        media(&pool, &drm_id).await,
        (
            "UNSUPPORTED".to_owned(),
            Some(MOBI_MEDIA_TYPE.to_owned()),
            Some("ERR_1001".to_owned()),
            0
        )
    );
    assert_eq!(
        media(&pool, &corrupt_id).await,
        (
            "ERROR".to_owned(),
            Some(MOBI_MEDIA_TYPE.to_owned()),
            Some("ERR_1005".to_owned()),
            0
        )
    );

    assert_webpub_and_download(&server, &good_id, &good, &inline).await;
    let key = api_key(&server, None).await;
    assert_kobo_downloads(&server, &key, &good_id, dir.path()).await;
    let lib_id = library["id"].as_str().expect("library id");
    assert_koreader_sync(&server, &pool, lib_id, &key, dir.path()).await;
}
