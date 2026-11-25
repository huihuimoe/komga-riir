//! Fonts for the EPUB reader (`FontsController`).

use std::fs;
use std::path::Path;

use reqwest::header::{
    ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE,
};
use reqwest::{Method, Response, StatusCode};
use serde_json::json;

use crate::support::TestServer;
use crate::support::openapi::checked;

const EMBEDDED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../komga/src/main/resources/embeddedFonts/OpenDyslexic/OpenDyslexic-Bold.woff2"
);

const CUSTOM_CSS: &str = r#"@font-face {
    font-family: 'Custom';
    src: url('Custom%20Italic.otf') format('opentype');
    font-weight: normal;
    font-style: italic;
}

@font-face {
    font-family: 'Custom';
    src: url('Custom-Bold.TTF') format('truetype'),url('Custom-Bold.woff2') format('woff2');
    font-weight: bold;
    font-style: normal;
}
"#;

fn write(path: &Path, contents: &[u8]) {
    fs::create_dir_all(path.parent().expect("Font directory")).expect("Create font directory");
    fs::write(path, contents).expect("Write font file");
}

fn header(response: &Response, name: &reqwest::header::HeaderName) -> String {
    response.headers()[name]
        .to_str()
        .expect("ASCII header")
        .to_owned()
}

async fn assert_font(response: Response, body: &[u8], media_type: &str, disposition: &str) {
    assert_eq!(response.status(), StatusCode::OK, "{disposition}");
    assert_eq!(header(&response, &CONTENT_TYPE), media_type);
    assert_eq!(header(&response, &CONTENT_DISPOSITION), disposition);
    assert_eq!(response.bytes().await.expect("Font body"), body);
}

#[tokio::test]
async fn fonts_are_listed_at_startup_and_served_anonymously() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let fonts = config_dir.path().join("fonts");
    write(&fonts.join("Custom/Custom-Bold.TTF"), b"ttf");
    write(&fonts.join("Custom/Custom-Bold.woff2"), b"woff2");
    write(&fonts.join("Custom/Custom Italic.otf"), b"otf");
    write(&fonts.join("Custom/notes.txt"), b"notes");
    fs::create_dir_all(fonts.join("Empty")).expect("Empty family");
    write(&fonts.join("loose.ttf"), b"not a family");
    let server = TestServer::start_claimed(config_dir.path()).await;
    let anonymous = |path: &str| server.request(Method::GET, path).send();

    let families = server
        .admin(Method::GET, "/api/v1/fonts/families")
        .send()
        .await
        .expect("Families");
    assert_eq!(
        checked(Method::GET, families).await,
        json!(["OpenDyslexic", "Custom", "Empty"])
    );
    let families = anonymous("/api/v1/fonts/families").await.expect("Families");
    assert_eq!(families.status(), StatusCode::UNAUTHORIZED);

    let embedded = fs::read(EMBEDDED).expect("Embedded font");
    let response = anonymous("/api/v1/fonts/resource/OpenDyslexic/OpenDyslexic-Bold.woff2").await;
    let disposition = "attachment; filename=\"OpenDyslexic-Bold.woff2\"";
    assert_font(
        response.expect("Embedded"),
        &embedded,
        "font/woff2",
        disposition,
    )
    .await;
    let response = anonymous("/api/v1/fonts/resource/Custom/Custom-Bold.TTF").await;
    let disposition = "attachment; filename=\"Custom-Bold.TTF\"";
    assert_font(response.expect("Custom"), b"ttf", "font/ttf", disposition).await;
    let response = anonymous("/api/v1/fonts/resource/Custom/Custom%20Italic.otf").await;
    let disposition = "attachment; filename=\"Custom Italic.otf\"";
    assert_font(response.expect("Spaced"), b"otf", "font/otf", disposition).await;

    let css = anonymous("/api/v1/fonts/resource/Custom/css")
        .await
        .expect("CSS");
    assert_font(
        css,
        CUSTOM_CSS.as_bytes(),
        "text/css",
        "attachment; filename=\"Custom.css\"",
    )
    .await;
    let css = anonymous("/api/v1/fonts/resource/Empty/css")
        .await
        .expect("CSS");
    assert_font(css, b"", "text/css", "attachment; filename=\"Empty.css\"").await;

    // Files appear only at startup.
    write(&fonts.join("Custom/Custom-Light.woff"), b"woff");
    for path in [
        "Custom/Custom-Light.woff",
        "Custom/notes.txt",
        "Unknown/Custom-Bold.TTF",
        "Unknown/css",
    ] {
        let response = anonymous(&format!("/api/v1/fonts/resource/{path}"))
            .await
            .expect("Missing font");
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
    let response = server
        .request(Method::GET, "/api/v1/fonts/resource/Custom/css")
        .basic_auth("nobody@example.com", Some("wrong"))
        .send()
        .await
        .expect("Bad credentials");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "credentials that are sent must be valid"
    );
    server.stop().await;
}

#[tokio::test]
async fn a_font_directory_replaces_the_embedded_family() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let fonts = config_dir.path().join("fonts");
    write(&fonts.join("OpenDyslexic/Mine.woff"), b"woff");
    write(&fonts.join("Another/Another.woff"), b"woff");
    let server = TestServer::start_claimed(config_dir.path()).await;

    let families = server
        .admin(Method::GET, "/api/v1/fonts/families")
        .send()
        .await
        .expect("Families");
    assert_eq!(
        checked(Method::GET, families).await,
        json!(["OpenDyslexic", "Another"])
    );
    let get = |path: &str| {
        server
            .request(
                Method::GET,
                &format!("/api/v1/fonts/resource/OpenDyslexic/{path}"),
            )
            .send()
    };
    let embedded = get("OpenDyslexic-Bold.woff2").await.expect("Embedded");
    assert_eq!(embedded.status(), StatusCode::NOT_FOUND);
    let mine = get("Mine.woff").await.expect("Replacement");
    assert_font(
        mine,
        b"woff",
        "font/woff",
        "attachment; filename=\"Mine.woff\"",
    )
    .await;
    server.stop().await;
}

#[tokio::test]
async fn font_ranges_follow_spring_resource_handling() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    write(
        &config_dir.path().join("fonts/Range/Range.woff"),
        b"0123456789",
    );
    let server = TestServer::start_claimed(config_dir.path()).await;
    let get = |path: &str, range: Option<&str>| {
        let request = server.request(Method::GET, &format!("/api/v1/fonts/resource/{path}"));
        match range {
            Some(range) => request.header(RANGE, range),
            None => request,
        }
        .send()
    };

    let whole = get("Range/Range.woff", None).await.expect("Whole font");
    assert_eq!(header(&whole, &ACCEPT_RANGES), "bytes");
    assert_font(
        whole,
        b"0123456789",
        "font/woff",
        "attachment; filename=\"Range.woff\"",
    )
    .await;

    let single = get("Range/Range.woff", Some("bytes=2-4"))
        .await
        .expect("Range");
    assert_eq!(single.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header(&single, &CONTENT_RANGE), "bytes 2-4/10");
    assert_eq!(header(&single, &CONTENT_TYPE), "font/woff");
    assert_eq!(header(&single, &CONTENT_LENGTH), "3");
    assert_eq!(single.bytes().await.expect("Body"), "234");

    let multiple = get("Range/Range.woff", Some("bytes=-3, 0-1"))
        .await
        .expect("Ranges");
    assert_eq!(multiple.status(), StatusCode::PARTIAL_CONTENT);
    let content_type = header(&multiple, &CONTENT_TYPE);
    let boundary = content_type
        .strip_prefix("multipart/byteranges; boundary=")
        .expect("Multipart type")
        .to_owned();
    let part = |range: &str, body: &str| {
        format!(
            "\r\n--{boundary}\r\nContent-Type: font/woff\r\nContent-Range: bytes {range}/10\r\n\r\n{body}"
        )
    };
    let expected = format!(
        "{}{}\r\n--{boundary}--",
        part("7-9", "789"),
        part("0-1", "01")
    );
    assert_eq!(multiple.text().await.expect("Body"), expected);

    // Spring still writes the whole resource.
    let unsatisfiable = get("Range/Range.woff", Some("bytes=10-"))
        .await
        .expect("416");
    assert_eq!(unsatisfiable.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(header(&unsatisfiable, &CONTENT_RANGE), "bytes */10");
    assert_eq!(unsatisfiable.bytes().await.expect("Body"), "0123456789");

    let css = get("Range/css", Some("bytes=0-10"))
        .await
        .expect("CSS range");
    assert_eq!(css.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(header(&css, &CONTENT_TYPE), "text/css");
    assert_eq!(css.text().await.expect("CSS"), "@font-face ");
    let embedded = fs::read(EMBEDDED).expect("Embedded font");
    let response = get("OpenDyslexic/OpenDyslexic-Bold.woff2", Some("bytes=-4"))
        .await
        .expect("Embedded range");
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        response.bytes().await.expect("Body"),
        embedded[embedded.len() - 4..]
    );
    server.stop().await;
}
