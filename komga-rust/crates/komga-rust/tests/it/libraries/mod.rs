//! Library CRUD over real HTTP and a file-backed database.

mod crud;
mod deletion;
mod empty_trash;
mod index_recovery;
mod nas;
mod permissions;
mod scan;
mod startup_scan;
mod task_recovery;
mod validation;

use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::{TestServer, openapi};

const LIBRARIES: &str = "/api/v1/libraries";

pub(crate) async fn admin_json(
    server: &TestServer,
    method: Method,
    path: &str,
    body: Option<&Value>,
    status: StatusCode,
) -> Value {
    let mut request = server.admin(method.clone(), path);
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("Library request failed");
    assert_eq!(response.status(), status, "{method} {path}");
    if status.is_success() {
        openapi::checked(method, response).await
    } else {
        crate::support::json(response).await
    }
}

pub(crate) async fn create(server: &TestServer, name: &str, root: &Path) -> Value {
    admin_json(
        server,
        Method::POST,
        LIBRARIES,
        Some(&json!({"name": name, "root": root})),
        StatusCode::OK,
    )
    .await
}

pub(crate) fn route(library: &Value) -> String {
    format!(
        "{LIBRARIES}/{}",
        library["id"].as_str().expect("Library ID must be a string")
    )
}
