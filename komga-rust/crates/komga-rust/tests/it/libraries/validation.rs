use komga_types::FileUrl;
use reqwest::{Method, StatusCode};
use serde_json::json;

use super::{LIBRARIES, admin_json, create, route};
use crate::support::{TestServer, db};

#[tokio::test]
async fn invalid_paths_names_and_options_do_not_modify_existing_library() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = directory.path().join("root");
    let other_root = directory.path().join("other");
    let child = root.join("child");
    std::fs::create_dir_all(&child).expect("Root and child");
    std::fs::create_dir(&other_root).expect("Other root");
    let file = directory.path().join("file");
    std::fs::write(&file, b"source").expect("File");
    let server = TestServer::start_claimed(directory.path()).await;
    let library = create(&server, "Original", &root).await;
    let other = create(&server, "Other", &other_root).await;
    let path = route(&other);
    for patch in [
        json!({"name":"Original"}),
        json!({"root":directory.path().join("missing")}),
        json!({"root":file}),
        json!({"root":child}),
        json!({"root":directory.path()}),
        json!({"root":root}),
        json!({"scanInterval":"MONTHLY"}),
        json!({"seriesCover":"UNKNOWN"}),
        json!({"name":" "}),
        json!({"root":"\t"}),
    ] {
        admin_json(
            &server,
            Method::PATCH,
            &path,
            Some(&patch),
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert_eq!(
            admin_json(&server, Method::GET, &path, None, StatusCode::OK).await,
            other
        );
    }
    for (name, path) in [
        ("Original", other_root),
        ("Child", child),
        ("Parent", directory.path().to_owned()),
        ("Missing", directory.path().join("missing")),
        ("File", file),
    ] {
        admin_json(
            &server,
            Method::POST,
            LIBRARIES,
            Some(&json!({"name":name, "root":path})),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    let missing = admin_json(
        &server,
        Method::POST,
        LIBRARIES,
        Some(&json!({"name":"Missing", "root":directory.path().join("no such [dir]")})),
        StatusCode::BAD_REQUEST,
    )
    .await;
    assert_eq!(
        missing["message"],
        format!(
            "Library root folder does not exist: {}no%20such%20%5Bdir%5D",
            FileUrl::directory(directory.path())
        )
    );
    for method in [Method::GET, Method::PATCH, Method::PUT, Method::DELETE] {
        admin_json(
            &server,
            method,
            &format!("{LIBRARIES}/missing"),
            Some(&json!({})),
            StatusCode::NOT_FOUND,
        )
        .await;
    }
    assert_eq!(
        admin_json(&server, Method::GET, &route(&library), None, StatusCode::OK).await,
        library
    );
    server.stop().await;
}

#[cfg(unix)]
#[tokio::test]
async fn canonical_path_checks_reject_symlink_and_dotdot_overlap() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = directory.path().join("root");
    let child = root.join("child");
    std::fs::create_dir_all(&child).expect("Root and child");
    let alias = directory.path().join("alias");
    std::os::unix::fs::symlink(&root, &alias).expect("Symlink");
    let server = TestServer::start_claimed(directory.path()).await;
    create(&server, "Original", &root).await;
    for path in [alias, child.join(".."), child.clone()] {
        admin_json(
            &server,
            Method::POST,
            LIBRARIES,
            Some(&json!({"name":"Alias", "root":path})),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
    // A library may move within its own root; it is excluded from overlap validation.
    let libraries = admin_json(&server, Method::GET, LIBRARIES, None, StatusCode::OK).await;
    admin_json(
        &server,
        Method::PATCH,
        &route(&libraries[0]),
        Some(&json!({"root":child})),
        StatusCode::NO_CONTENT,
    )
    .await;
    let libraries = admin_json(&server, Method::GET, LIBRARIES, None, StatusCode::OK).await;
    assert_eq!(libraries[0]["root"], json!(child));
    server.stop().await;
}

#[tokio::test]
async fn unavailable_library_root_does_not_block_other_libraries() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let gone = directory.path().join("gone");
    let root = directory.path().join("root");
    std::fs::create_dir(&gone).expect("Gone root");
    std::fs::create_dir(&root).expect("Root");
    let server = TestServer::start_claimed(directory.path()).await;
    create(&server, "Gone", &gone).await;
    std::fs::remove_dir(&gone).expect("Remove gone root");

    let created = admin_json(
        &server,
        Method::POST,
        LIBRARIES,
        Some(&json!({"name":"Root", "root":format!("{}/", root.display())})),
        StatusCode::OK,
    )
    .await;
    assert_eq!(created["root"], json!(root));
    admin_json(
        &server,
        Method::PATCH,
        &route(&created),
        Some(&json!({"name":"Renamed"})),
        StatusCode::NO_CONTENT,
    )
    .await;
    // The unavailable root still takes part in the overlap check, literally.
    admin_json(
        &server,
        Method::PATCH,
        &route(&created),
        Some(&json!({"root":directory.path()})),
        StatusCode::BAD_REQUEST,
    )
    .await;
    server.stop().await;
}

#[tokio::test]
async fn failed_exclusion_write_rolls_back_options_and_exclusions() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = directory.path().join("root");
    std::fs::create_dir(&root).expect("Root");
    let server = TestServer::start_claimed(directory.path()).await;
    let library = create(&server, "Original", &root).await;
    let pool = db::main_pool(directory.path()).await;
    db::inject_failure(&pool, "BEFORE INSERT ON LIBRARY_EXCLUSIONS").await;
    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"name":"Changed", "scanDirectoryExclusions":["private"]})),
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await;
    assert_eq!(
        admin_json(&server, Method::GET, &route(&library), None, StatusCode::OK).await,
        library
    );
    pool.close().await;
    server.stop().await;
}
