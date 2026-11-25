use super::{LIBRARIES, admin_json, create, route};
use crate::support::{TestServer, db};
use komga_types::FileUrl;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

fn options() -> Value {
    json!({
        "importComicInfoBook": false,
        "importComicInfoSeries": false,
        "importComicInfoCollection": false,
        "importComicInfoReadList": false,
        "importComicInfoSeriesAppendVolume": false,
        "importEpubBook": false,
        "importEpubSeries": false,
        "importMylarSeries": false,
        "importLocalArtwork": false,
        "importBarcodeIsbn": false,
        "scanForceModifiedTime": true,
        "scanOnStartup": true,
        "scanInterval": "WEEKLY",
        "scanCbx": false,
        "scanPdf": false,
        "scanEpub": false,
        "scanDirectoryExclusions": [
            "a*",
            "z*"
        ],
        "repairExtensions": true,
        "convertToCbz": true,
        "emptyTrashAfterScan": true,
        "seriesCover": "FIRST_UNREAD_OR_LAST",
        "hashFiles": false,
        "hashPages": true,
        "hashKoreader": true,
        "analyzeDimensions": false,
        "oneshotsDirectory": " one shots "
    })
}

#[tokio::test]
async fn creation_options_survive_restart_and_encoded_storage() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = directory.path().join("库 % # [spaces]");
    std::fs::create_dir(&root).expect("Library root");
    let server = TestServer::start_claimed(directory.path()).await;
    let mut expected = options();
    expected["name"] = json!("Non-default options");
    expected["root"] = json!(root);
    let library = admin_json(
        &server,
        Method::POST,
        LIBRARIES,
        Some(&expected),
        StatusCode::OK,
    )
    .await;
    expected["id"] = library["id"].clone();
    expected["unavailable"] = json!(false);
    assert_eq!(library, expected);
    assert_eq!(library["id"].as_str().expect("ID").len(), 13);
    let path = route(&library);
    server.stop().await;
    // Java `Path.toUri().toURL()`: `file:/`, directory `/`, percent-encoded specials (and UTF-8
    // bytes on Unix; Windows keeps non-ASCII characters unescaped).
    #[cfg(unix)]
    let tail = "%E5%BA%93%20%25%20%23%20%5Bspaces%5D/";
    #[cfg(not(unix))]
    let tail = "库%20%25%20%23%20%5Bspaces%5D/";
    let stored = format!("{}{tail}", FileUrl::directory(directory.path()));
    let pool = db::main_pool(directory.path()).await;
    assert_eq!(db::library_root(&pool).await, stored);
    // Earlier builds wrote the equivalent `file:///` spelling.
    db::seed::library_root(&pool, &stored.replacen("file:", "file://", 1)).await;
    pool.close().await;
    let server = TestServer::start(directory.path()).await;
    assert_eq!(
        admin_json(&server, Method::GET, &path, None, StatusCode::OK).await,
        expected
    );
    server.stop().await;
}

#[tokio::test]
async fn partial_patch_and_deprecated_put_preserve_absent_and_clear_nullable_fields() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let root = directory.path().join("root");
    std::fs::create_dir(&root).expect("Library root");
    let server = TestServer::start_claimed(directory.path()).await;
    let mut expected = create(&server, "Original", &root).await;
    let path = route(&expected);
    let options = options();
    admin_json(
        &server,
        Method::PATCH,
        &path,
        Some(&options),
        StatusCode::NO_CONTENT,
    )
    .await;
    for (key, value) in options.as_object().expect("Options") {
        expected[key] = value.clone();
    }
    let patch = json!({"name": null, "root": null, "scanPdf": null, "oneshotsDirectory": null, "scanDirectoryExclusions": null});
    admin_json(
        &server,
        Method::PATCH,
        &path,
        Some(&patch),
        StatusCode::NO_CONTENT,
    )
    .await;
    expected["oneshotsDirectory"] = Value::Null;
    expected["scanDirectoryExclusions"] = json!([]);
    assert_eq!(
        admin_json(&server, Method::GET, &path, None, StatusCode::OK).await,
        expected
    );
    admin_json(
        &server,
        Method::PUT,
        &path,
        Some(&json!({"name":"Renamed","oneshotsDirectory":"  "})),
        StatusCode::NO_CONTENT,
    )
    .await;
    expected["name"] = json!("Renamed");
    server.stop().await;
    let server = TestServer::start(directory.path()).await;
    assert_eq!(
        admin_json(&server, Method::GET, &path, None, StatusCode::OK).await,
        expected
    );
    server.stop().await;
}
