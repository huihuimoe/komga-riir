//! Full-text searches of the lists: every hit of the index, filtered, ordered, counted, and paged
//! by the database as the user may see it.

mod catalog;
mod sets;

use reqwest::{Method, StatusCode};
use serde_json::Value;

use crate::read_lists::{Fixture, send};
use crate::support::openapi;

const LIBRARY_A_USER: &str = "a@example.com";

async fn get(f: &Fixture, email: Option<&str>, path: &str) -> Value {
    answer(f, email, Method::GET, path, None).await
}

async fn post(f: &Fixture, email: Option<&str>, path: &str, body: &Value) -> Value {
    answer(f, email, Method::POST, path, Some(body)).await
}

/// The successful answer to `email`, or to the admin.
async fn answer(
    f: &Fixture,
    email: Option<&str>,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Value {
    let response = send(&f.server, email, method.clone(), path, body).await;
    assert_eq!(response.status(), StatusCode::OK, "{path} {body:?}");
    openapi::checked(method, response).await
}

/// The `field` of every item of `page`, in order.
fn values<'a>(page: &'a Value, field: &str) -> Vec<&'a str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|item| item[field].as_str().expect("A string field"))
        .collect()
}
