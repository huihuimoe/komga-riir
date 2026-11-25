//! Uploading and listing posters of books, series and read lists.

use std::io::Cursor;

use image::{ImageFormat, Rgb, RgbImage};
use reqwest::multipart::{Form, Part};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde_json::Value;

use crate::support::{TestServer, openapi};

/// A blue PNG.
pub fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    RgbImage::from_pixel(width, height, Rgb([0, 0, 200]))
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("Encode poster");
    bytes
}

/// The multipart form of an upload.
pub fn file(bytes: Vec<u8>) -> Form {
    Form::new().part("file", Part::bytes(bytes).file_name("poster.png"))
}

pub fn selected_ids(posters: &[Value]) -> Vec<&str> {
    posters
        .iter()
        .filter(|poster| poster["selected"] == true)
        .map(|poster| poster["id"].as_str().expect("Poster ID"))
        .collect()
}

/// The posters a poster list route returns to the admin, checked against the baseline OpenAPI.
pub async fn list_at(server: &TestServer, path: &str) -> Vec<Value> {
    let response = server
        .admin(Method::GET, path)
        .send()
        .await
        .expect("List request");
    assert_eq!(response.status(), StatusCode::OK);
    let Value::Array(posters) = openapi::checked(Method::GET, response).await else {
        panic!("A JSON array of posters");
    };
    posters
}

pub async fn status_and_body(request: RequestBuilder) -> (StatusCode, Value) {
    let response = request.send().await.expect("Request");
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}
