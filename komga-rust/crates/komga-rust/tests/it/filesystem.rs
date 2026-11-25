//! Host filesystem browsing through the real HTTP server, with no database fixtures.

use std::fs;
use std::path::Path;

use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

use crate::support::{TestServer, openapi};

const ROUTE: &str = "/api/v1/filesystem";

async fn listing(server: &TestServer, body: Option<&Value>) -> Value {
    let mut request = server.admin(Method::POST, ROUTE);
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.expect("Directory listing request");
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::POST, response).await
}

async fn assert_error(response: Response, status: StatusCode, message: &str) {
    assert_eq!(response.status(), status);
    let body = crate::support::json(response).await;
    assert_eq!(body["path"], ROUTE);
    assert_eq!(body["message"], message);
    assert!(
        body.get("directories").is_none(),
        "Failures must not look like empty listings"
    );
}

fn entries(root: &Path, names: &[&str], entry_type: &str) -> Value {
    names
        .iter()
        .map(|name| {
            json!({
                "type": entry_type, "name": name, "path": root.join(name)
            })
        })
        .collect()
}

#[tokio::test]
async fn optional_body_defaults_to_roots_without_parent() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let roots = listing(&server, None).await;
    #[cfg(not(windows))]
    assert_eq!(
        roots,
        json!({"directories": [{"type":"directory", "name":"/", "path":"/"}], "files":[]})
    );
    for body in [json!({}), Value::Null, json!({"path":"", "showFiles":true})] {
        assert_eq!(listing(&server, Some(&body)).await, roots);
    }
    let response = server
        .admin(Method::POST, ROUTE)
        .header("Content-Type", "application/json")
        .send()
        .await
        .expect("Empty JSON body");
    assert_eq!(openapi::checked(Method::POST, response).await, roots);
    assert!(roots.get("parent").is_none());
    server.stop().await;
}

#[tokio::test]
async fn listing_partitions_visible_entries_and_uses_simple_case_order() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = directory.path().join("库 % # spaces");
    fs::create_dir(&root).expect("Browse directory");
    let directories = ["alpha", "ıbis", "İota", "Zulu"];
    // No "ss" sibling for "ß": case-insensitive APFS stores both as one file. "ß" after "st"
    // still rules out full case folding.
    let files = ["ı-a.cbz", "i-b.cbz", "İ-c.cbz", "st.cbz", "ß.cbz"];
    for name in directories.iter().rev() {
        fs::create_dir(root.join(name)).expect("Child directory");
    }
    for name in files.iter().rev() {
        fs::write(root.join(name), b"book").expect("Child file");
    }
    #[cfg(not(windows))]
    {
        fs::create_dir(root.join(".hidden-directory")).expect("Hidden directory");
        fs::write(root.join(".hidden-file"), b"hidden").expect("Hidden file");
    }
    let server = TestServer::start_claimed(directory.path()).await;
    let folders = entries(&root, &directories, "directory");
    assert_eq!(
        listing(&server, Some(&json!({"path":root}))).await,
        json!({"parent":directory.path(), "directories":folders, "files":[]})
    );
    assert_eq!(
        listing(&server, Some(&json!({"path":root, "showFiles":true}))).await,
        json!({"parent":directory.path(), "directories":folders, "files":entries(&root, &files, "file")})
    );
    server.stop().await;
}

#[tokio::test]
async fn files_and_missing_leaf_paths_list_the_requested_parent() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = directory.path().join("browse");
    fs::create_dir_all(root.join("child")).expect("Browse directory");
    fs::write(root.join("book.cbz"), b"book").expect("Book file");
    let server = TestServer::start_claimed(directory.path()).await;
    for path in [root.join("book.cbz"), root.join("missing.cbz")] {
        let body = listing(&server, Some(&json!({"path":path, "showFiles":true}))).await;
        assert_eq!(
            body,
            json!({
                "parent":root,
                "directories":entries(&root, &["child"], "directory"),
                "files":entries(&root, &["book.cbz"], "file")
            })
        );
    }
    // ADMIN browsing is host-wide; an absolute .. is not an escape from a library sandbox.
    let path = root.join("child").join("..");
    assert_eq!(
        listing(&server, Some(&json!({"path":path}))).await,
        json!({"parent":root.join("child"), "directories":entries(&path, &["child"], "directory"), "files":[]})
    );
    server.stop().await;
}

#[tokio::test]
async fn invalid_or_unreadable_paths_return_errors_instead_of_empty_lists() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let file = directory.path().join("book.cbz");
    fs::write(&file, b"book").expect("File");
    let server = TestServer::start_claimed(directory.path()).await;
    for (path, message) in [
        ("../outside".to_owned(), "Path must be absolute"),
        (".".to_owned(), "Path must be absolute"),
        (
            directory
                .path()
                .join("missing/child")
                .to_string_lossy()
                .into_owned(),
            "Path does not exist",
        ),
        (
            file.join("child").to_string_lossy().into_owned(),
            "Path does not exist",
        ),
    ] {
        let response = server
            .admin(Method::POST, ROUTE)
            .json(&json!({"path":path}))
            .send()
            .await
            .expect("Invalid path request");
        assert_error(response, StatusCode::BAD_REQUEST, message).await;
    }
    let response = server
        .admin(Method::POST, ROUTE)
        .json(&json!({"path":"/bad\u{0000}path"}))
        .send()
        .await
        .expect("NUL path request");
    assert_error(
        response,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error",
    )
    .await;
    server.stop().await;
}

#[tokio::test]
async fn optional_body_still_rejects_malformed_or_unsupported_content() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    for body in [
        "{",
        "[]",
        "\"/\"",
        "{\"path\":null}",
        "{\"showFiles\":null}",
        " ",
    ] {
        let response = server
            .admin(Method::POST, ROUTE)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .expect("Malformed body request");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error = crate::support::json(response).await;
        assert!(
            error["message"]
                .as_str()
                .expect("Error detail")
                .starts_with("JSON parse error:")
        );
    }
    for request in [
        server.admin(Method::POST, ROUTE).body("{}"),
        server
            .admin(Method::POST, ROUTE)
            .header("Content-Type", "text/plain")
            .body("{}"),
        server
            .admin(Method::POST, ROUTE)
            .header("Content-Type", "text/plain"),
        server
            .admin(Method::POST, ROUTE)
            .header("Content-Type", "invalid"),
    ] {
        let response = request.send().await.expect("Unsupported body request");
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(
            response.headers()["Accept"],
            "application/json, application/*+json"
        );
    }
    let response = server
        .admin(Method::POST, ROUTE)
        .header("Content-Type", "application/komga+json")
        .body("{}")
        .send()
        .await
        .expect("Vendor JSON request");
    assert_eq!(response.status(), StatusCode::OK);
    openapi::checked(Method::POST, response).await;
    server.stop().await;
}

#[tokio::test]
async fn only_admins_can_browse_and_body_binding_precedes_method_security() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&json!({"email":"reader@example.com", "password":"readerpass"}))
        .send()
        .await
        .expect("Create reader");
    assert_eq!(response.status(), StatusCode::CREATED);
    for body in [
        json!({}),
        json!({"path":"../outside"}),
        json!({"path":"/bad\u{0000}path"}),
    ] {
        let response = server
            .request(Method::POST, ROUTE)
            .json(&body)
            .send()
            .await
            .expect("Anonymous request");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let response = server
            .request(Method::POST, ROUTE)
            .basic_auth("reader@example.com", Some("readerpass"))
            .json(&body)
            .send()
            .await
            .expect("Reader request");
        assert_error(response, StatusCode::FORBIDDEN, "Forbidden").await;
    }
    let response = server
        .request(Method::POST, ROUTE)
        .basic_auth("reader@example.com", Some("readerpass"))
        .header("Content-Type", "application/json")
        .body("{")
        .send()
        .await
        .expect("Malformed reader body");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    server.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_follow_targets_and_loops_are_files() {
    use std::os::unix::fs::symlink;

    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = directory.path().join("links");
    let target = directory.path().join("outside");
    fs::create_dir(&root).expect("Link directory");
    fs::create_dir(&target).expect("Target directory");
    fs::write(target.join("book.cbz"), b"book").expect("Target file");
    symlink(&target, root.join("directory")).expect("Directory symlink");
    symlink(target.join("book.cbz"), root.join("file.cbz")).expect("File symlink");
    symlink(target.join("missing"), root.join("dangling.cbz")).expect("Dangling symlink");
    let server = TestServer::start_claimed(directory.path()).await;
    assert_eq!(
        listing(&server, Some(&json!({"path":root, "showFiles":true}))).await,
        json!({"parent":directory.path(), "directories":entries(&root, &["directory"], "directory"),
            "files":entries(&root, &["dangling.cbz", "file.cbz"], "file")})
    );
    let alias = root.join("directory");
    assert_eq!(
        listing(&server, Some(&json!({"path":alias, "showFiles":true}))).await,
        json!({"parent":root, "directories":[], "files":entries(&alias, &["book.cbz"], "file")})
    );
    // A loop is not a directory to `Files.isDirectory`, so it lists its parent like a file.
    symlink(root.join("loop"), root.join("loop")).expect("Symlink loop");
    assert_eq!(
        listing(
            &server,
            Some(&json!({"path":root.join("loop"), "showFiles":true}))
        )
        .await,
        json!({"parent":root, "directories":entries(&root, &["directory"], "directory"),
            "files":entries(&root, &["dangling.cbz", "file.cbz", "loop"], "file")})
    );
    server.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn unreadable_paths_fail_the_listing_but_unreadable_link_targets_are_files() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let directory = komga_test_support::tempdir().expect("Data directory");
    let denied = directory.path().join("denied");
    let alias = directory.path().join("alias");
    fs::create_dir(&denied).expect("Denied directory");
    symlink(&denied, &alias).expect("Denied directory alias");
    let readable = directory.path().join("readable");
    fs::create_dir(&readable).expect("Readable directory");
    fs::create_dir(denied.join("inner")).expect("Directory behind the denied one");
    symlink(denied.join("inner"), readable.join("behind")).expect("Link behind denied");
    let server = TestServer::start_claimed(directory.path()).await;
    let permissions = fs::metadata(&denied).expect("Permissions").permissions();
    fs::set_permissions(&denied, fs::Permissions::from_mode(0o0)).expect("Deny access");
    // An entry whose target cannot be read is a file, as in `Files.isDirectory`.
    let behind = server
        .admin(Method::POST, ROUTE)
        .json(&json!({"path":readable, "showFiles":true}))
        .send()
        .await;
    let mut responses = Vec::new();
    for path in [denied.clone(), denied.join("missing"), alias] {
        responses.push(
            server
                .admin(Method::POST, ROUTE)
                .json(&json!({"path":path}))
                .send()
                .await,
        );
    }
    fs::set_permissions(&denied, permissions).expect("Restore permissions before assertions");
    let behind = behind.expect("Listing with an unreadable link target");
    assert_eq!(behind.status(), StatusCode::OK);
    let behind = crate::support::json(behind).await;
    assert_eq!(behind["files"], entries(&readable, &["behind"], "file"));
    for response in responses {
        assert_error(
            response.expect("Permission request"),
            StatusCode::BAD_REQUEST,
            "Path does not exist",
        )
        .await;
    }
    server.stop().await;
}
