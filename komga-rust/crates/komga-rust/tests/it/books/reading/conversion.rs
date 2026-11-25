//! Pages converted to JPEG or PNG on request, and page thumbnails.

use image::{GenericImageView, ImageFormat, Rgba, RgbaImage};
use reqwest::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE, IF_MODIFIED_SINCE, LAST_MODIFIED,
};
use reqwest::{Method, StatusCode};

use serde_json::json;

use super::access::create_reader;
use super::{CACHE_PRIVATE, disposition, fetch};
use crate::books::{GIF, JPEG, PNG, library_with_books};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;

const FIXTURES: &str = "komga-image/tests/fixtures";

fn image_fixture(name: &str) -> Vec<u8> {
    cbz::fixture(&format!("{FIXTURES}/{name}"))
}

fn png(image: &RgbaImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("Encode test PNG");
    bytes
}

/// A 4x2 PNG, transparent but for an opaque black first pixel.
fn transparent_png() -> Vec<u8> {
    let mut image = RgbaImage::from_pixel(4, 2, Rgba([0, 0, 0, 0]));
    image.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
    png(&image)
}

/// `photo.png` whose header claims 12,000 x 12,000 pixels, over the decoding limit.
fn oversized_png() -> Vec<u8> {
    let mut png = image_fixture("photo.png");
    png[16..24].copy_from_slice(&[0, 0, 0x2e, 0xe0, 0, 0, 0x2e, 0xe0]);
    let crc = crc32fast::hash(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

/// The bare codestream inside `photo.jp2`, from its SOC marker.
fn jpeg_2000_codestream() -> Vec<u8> {
    let jp2 = image_fixture("photo.jp2");
    let start = jp2
        .windows(4)
        .position(|window| window == b"\xff\x4f\xff\x51")
        .expect("JP2 fixture holds a codestream");
    jp2[start..].to_vec()
}

/// A Photoshop header, which no decoder reads.
const PHOTOSHOP: &[u8] = b"8BPS\0\x01\0\0\0\0\0\0\0\x03\0\0\0\x14\0\0\0\x30\0\x08\0\x03";

fn decoded(body: &[u8], format: ImageFormat) -> image::DynamicImage {
    image::load_from_memory_with_format(body, format).expect("Converted page decodes")
}

#[tokio::test]
async fn pages_are_converted_to_jpeg_or_png() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let png = cbz::fixture(PNG);
    let truncated = png[..png.len() / 2].to_vec();
    let book_file = cbz::zip(&[
        ("01.bmp", STORED, &image_fixture("photo.bmp")),
        ("02.gif", STORED, &cbz::fixture(GIF)),
        ("03.jpg", STORED, &image_fixture("photo.jpg")),
        ("04.png", STORED, &png),
        ("05.tiff", STORED, &image_fixture("photo.tiff")),
        ("06.webp", STORED, &image_fixture("lossy.webp")),
        ("07.webp", STORED, &image_fixture("animated.webp")),
        ("08.png", STORED, &transparent_png()),
        ("09.jp2", STORED, &image_fixture("photo.jp2")),
        ("10.png", STORED, &truncated),
        ("11.png", STORED, &oversized_png()),
        ("12.jpg", STORED, &cbz::fixture(JPEG)),
        ("13.jxl", STORED, &image_fixture("photo.jxl")),
        ("14.heic", STORED, &image_fixture("photo.heic")),
        ("15.avif", STORED, &image_fixture("photo.avif")),
        ("16.j2c", STORED, &jpeg_2000_codestream()),
        ("17.jbig2", STORED, &image_fixture("photo.jbig2")),
        ("18.pcx", STORED, &image_fixture("photo.pcx")),
        ("19.psd", STORED, PHOTOSHOP),
    ]);
    let (server, _, pool) =
        library_with_books(directory.path(), &[("Série é.cbz", book_file)]).await;
    let (book_id, _) = book(&pool, "Série é").await;
    let pages = format!("/api/v1/books/{book_id}/pages");

    for number in (1..=7).chain([9]).chain(13..=18) {
        for (convert, media_type, format, extension) in [
            ("jpeg", "image/jpeg", ImageFormat::Jpeg, "jpg"),
            ("PNG", "image/png", ImageFormat::Png, "png"),
        ] {
            let route = format!("{pages}/{number}?convert={convert}");
            let page = fetch(server.admin(Method::GET, &route)).await;
            assert_eq!(page.status, StatusCode::OK, "{route}");
            assert_eq!(page.header(&CONTENT_TYPE), media_type, "{route}");
            assert_eq!(
                page.header(&CONTENT_DISPOSITION),
                disposition(&format!("{number}.{extension}")),
                "{route}"
            );
            assert_eq!(
                decoded(&page.body, format).dimensions(),
                (48, 20),
                "{route}"
            );
        }
    }

    // JPEG has no alpha: transparency is laid on white; PNG keeps it.
    let jpeg = fetch(server.admin(Method::GET, &format!("{pages}/8?convert=jpeg"))).await;
    let [red, green, blue] = decoded(&jpeg.body, ImageFormat::Jpeg)
        .to_rgb8()
        .get_pixel(3, 1)
        .0;
    assert!(
        red > 250 && green > 250 && blue > 250,
        "{red} {green} {blue}"
    );
    let png_page = fetch(server.admin(Method::GET, &format!("{pages}/8?convert=png"))).await;
    let converted = decoded(&png_page.body, ImageFormat::Png).to_rgba8();
    assert_eq!(converted.get_pixel(3, 1).0, [0, 0, 0, 0]);
    assert_eq!(converted.get_pixel(0, 0).0, [0, 0, 0, 255]);

    // A format that cannot be decoded is served as stored, and refused for conversion.
    let stored = fetch(server.admin(Method::GET, &format!("{pages}/19"))).await;
    assert_eq!(stored.status, StatusCode::OK);
    assert_eq!(stored.header(&CONTENT_TYPE), "image/vnd.adobe.photoshop");
    assert!(stored.body == PHOTOSHOP, "stored Photoshop image");
    let refused = fetch(server.admin(Method::GET, &format!("{pages}/19?convert=png"))).await;
    assert_eq!(refused.status, StatusCode::NOT_FOUND);
    assert_eq!(
        refused.message(),
        format!(
            "Convert page #19 of book {book_id} from image/vnd.adobe.photoshop to image/png: \
             unsupported read format image/vnd.adobe.photoshop"
        )
    );

    // Corrupt and oversized pages fail without a partial image.
    for number in [10, 11] {
        let route = format!("{pages}/{number}?convert=jpeg");
        let failed = fetch(server.admin(Method::GET, &route)).await;
        assert_eq!(failed.status, StatusCode::INTERNAL_SERVER_ERROR, "{route}");
    }

    // A page in the requested format is not re-encoded.
    let same = fetch(server.admin(Method::GET, &format!("{pages}/12?convert=jpeg"))).await;
    assert!(same.body == cbz::fixture(JPEG), "page served as stored");
    server.stop().await;
}

#[tokio::test]
async fn page_thumbnails_are_jpeg_within_300_pixels() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let png_page = cbz::fixture(PNG);
    let truncated = png_page[..png_page.len() / 2].to_vec();
    let tall = png(&RgbaImage::from_pixel(600, 900, Rgba([0, 0, 255, 255])));
    let book_file = cbz::zip(&[
        ("1.png", STORED, &tall),
        ("2.png", STORED, &png_page),
        ("3.jpg", STORED, &image_fixture("photo.jpg")),
        ("4.jp2", STORED, &image_fixture("photo.jp2")),
        ("5.png", STORED, &truncated),
        ("6.jxl", STORED, &image_fixture("photo.jxl")),
        ("7.heic", STORED, &image_fixture("photo.heic")),
        ("8.avif", STORED, &image_fixture("photo.avif")),
        ("9.psd", STORED, PHOTOSHOP),
        ("10.jbig2", STORED, &image_fixture("photo.jbig2")),
        ("11.pcx", STORED, &image_fixture("photo.pcx")),
    ]);
    let (server, _, pool) = library_with_books(directory.path(), &[("book.cbz", book_file)]).await;
    let (book_id, _) = book(&pool, "book").await;
    let pages = format!("/api/v1/books/{book_id}/pages");

    // Any user who sees the book gets thumbnails, without PAGE_STREAMING.
    let reader = create_reader(&server, "reader@example.com", json!({"roles": []})).await;
    for (number, size) in [
        (1, (200, 300)),
        (2, (48, 20)),
        (4, (48, 20)),
        (6, (48, 20)),
        (7, (48, 20)),
        (8, (48, 20)),
        (10, (48, 20)),
        (11, (48, 20)),
    ] {
        let route = format!("{pages}/{number}/thumbnail");
        let thumbnail = fetch(reader.get(&server, &route)).await;
        assert_eq!(thumbnail.status, StatusCode::OK, "{route}");
        assert_eq!(thumbnail.header(&CONTENT_TYPE), "image/jpeg", "{route}");
        assert_eq!(thumbnail.header(&CONTENT_DISPOSITION), "", "{route}");
        assert_eq!(thumbnail.header(&CACHE_CONTROL), CACHE_PRIVATE, "{route}");
        let image = decoded(&thumbnail.body, ImageFormat::Jpeg);
        assert_eq!(image.dimensions(), size, "{route}");
    }
    let page = fetch(reader.get(&server, &format!("{pages}/1"))).await;
    assert_eq!(
        page.status,
        StatusCode::FORBIDDEN,
        "the page itself needs the role"
    );

    // A small JPEG is served as stored; the book's date answers conditional requests.
    let stored = fetch(server.admin(Method::GET, &format!("{pages}/3/thumbnail"))).await;
    assert!(
        stored.body == image_fixture("photo.jpg"),
        "small JPEG as stored"
    );
    let conditional = server
        .admin(Method::GET, &format!("{pages}/3/thumbnail"))
        .header(IF_MODIFIED_SINCE, stored.header(&LAST_MODIFIED));
    assert_eq!(fetch(conditional).await.status, StatusCode::NOT_MODIFIED);

    for (route, status) in [
        ("/5/thumbnail", StatusCode::INTERNAL_SERVER_ERROR),
        ("/9/thumbnail", StatusCode::INTERNAL_SERVER_ERROR),
        ("/12/thumbnail", StatusCode::BAD_REQUEST),
        ("/one/thumbnail", StatusCode::BAD_REQUEST),
    ] {
        let response = fetch(server.admin(Method::GET, &format!("{pages}{route}"))).await;
        assert_eq!(response.status, status, "{route}");
    }
    let missing = fetch(server.admin(Method::GET, "/api/v1/books/0/pages/1/thumbnail")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    let outsider = create_reader(
        &server,
        "outsider@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;
    let denied = fetch(outsider.get(&server, &format!("{pages}/1/thumbnail"))).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    server.stop().await;
}
