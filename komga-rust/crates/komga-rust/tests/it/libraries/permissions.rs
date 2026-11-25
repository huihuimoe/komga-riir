use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{LIBRARIES, admin_json, create, route};
use crate::support::{TestServer, json, openapi};

const EMAIL: &str = "reader@example.com";
const PASSWORD: &str = "reader-password";

async fn reader(server: &TestServer, allowed: &Value) {
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&json!({
            "email": EMAIL, "password": PASSWORD,
            "sharedLibraries": {"all": false, "libraryIds": [allowed["id"]]}
        }))
        .send()
        .await
        .expect("Create reader");
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn libraries_enforce_sharing_root_redaction_and_lowercase_sorting() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let mut libraries = Vec::new();
    for name in ["z-last", "B-middle", "a-first"] {
        let root = directory.path().join(name);
        std::fs::create_dir(&root).expect("Root");
        libraries.push(create(&server, name, &root).await);
    }
    reader(&server, &libraries[1]).await;
    let all = admin_json(&server, Method::GET, LIBRARIES, None, StatusCode::OK).await;
    assert_eq!(all, json!([libraries[2], libraries[1], libraries[0]]));
    let response = server
        .request(Method::GET, LIBRARIES)
        .basic_auth(EMAIL, Some(PASSWORD))
        .send()
        .await
        .expect("Reader list");
    let mut visible = libraries[1].clone();
    visible["root"] = json!("");
    assert_eq!(
        openapi::checked(Method::GET, response).await,
        json!([visible])
    );
    let response = server
        .request(Method::GET, &route(&libraries[1]))
        .basic_auth(EMAIL, Some(PASSWORD))
        .send()
        .await
        .expect("Shared library");
    assert_eq!(openapi::checked(Method::GET, response).await, visible);
    for (path, status) in [
        (route(&libraries[0]), StatusCode::FORBIDDEN),
        (format!("{LIBRARIES}/missing"), StatusCode::NOT_FOUND),
    ] {
        let response = server
            .request(Method::GET, &path)
            .basic_auth(EMAIL, Some(PASSWORD))
            .send()
            .await
            .expect("Hidden or missing library");
        assert_eq!(response.status(), status);
    }
    server.stop().await;
}

#[tokio::test]
async fn library_writes_reject_readers_and_validate_bodies_before_method_security() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = directory.path().join("root");
    std::fs::create_dir(&root).expect("Root");
    let server = TestServer::start_claimed(directory.path()).await;
    let library = create(&server, "Library", &root).await;
    reader(&server, &library).await;
    let path = route(&library);
    for (method, target, body) in [
        (Method::POST, LIBRARIES, json!({"name":"New", "root":root})),
        (Method::PATCH, path.as_str(), json!({"name":"Changed"})),
        (Method::PUT, path.as_str(), json!({"name":"Changed"})),
        (Method::DELETE, path.as_str(), Value::Null),
    ] {
        let response = server
            .request(method.clone(), target)
            .json(&body)
            .send()
            .await
            .expect("Anonymous request");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = server
            .request(method, target)
            .basic_auth(EMAIL, Some(PASSWORD))
            .json(&body)
            .send()
            .await
            .expect("Reader write");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(json(response).await["message"], "Forbidden");
    }
    for method in [Method::POST, Method::PATCH, Method::PUT] {
        let target = if method == Method::POST {
            LIBRARIES
        } else {
            &path
        };
        let response = server
            .request(method, target)
            .basic_auth(EMAIL, Some(PASSWORD))
            .json(&json!({"name":" ", "root":" "}))
            .send()
            .await
            .expect("Invalid reader write");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = json(response).await;
        assert_eq!(body["violations"].as_array().expect("Violations").len(), 2);
    }
    assert_eq!(
        admin_json(&server, Method::GET, &path, None, StatusCode::OK).await,
        library
    );
    server.stop().await;
}
