//! `deleteEmptyReadLists` and `deleteEmptyCollections`: read lists and collections left without
//! members go after scans and trash emptying, with their posters, when the settings ask.

use std::fs;

use image::{GenericImageView, ImageFormat};
use komga_types::SearchEntity;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::{READ_LISTS, fixture, send};
use crate::support::posters::{file, list_at, png, selected_ids};
use crate::support::search::{hits, wait_for_index};
use crate::support::tasks::{wait_for_scan_work, wait_for_tasks};
use crate::support::{TestServer, json};

const COLLECTIONS: &str = "/api/v1/collections";

/// Creates the set `name` under `root` with one member, returning its path.
async fn create(
    server: &TestServer,
    root: &str,
    members: &str,
    name: &str,
    member: &str,
) -> String {
    let body = json!({"name": name, members: [member]});
    let created = json(send(server, None, Method::POST, root, Some(&body)).await).await;
    format!("{root}/{}", created["id"].as_str().expect("Set ID"))
}

async fn update_settings(server: &TestServer, settings: Value) {
    let code = status(server, Method::PATCH, "/api/v1/settings", Some(&settings)).await;
    assert_eq!(code, StatusCode::NO_CONTENT);
}

async fn assert_black_poster(server: &TestServer, path: &str) {
    let response = send(
        server,
        None,
        Method::GET,
        &format!("{path}/thumbnail"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mosaic = response.bytes().await.expect("Mosaic");
    let mosaic = image::load_from_memory_with_format(&mosaic, ImageFormat::Jpeg).expect("JPEG");
    assert!(
        mosaic
            .pixels()
            .all(|(_, _, pixel)| pixel.0[..3].iter().all(|channel| *channel < 16)),
        "a black poster"
    );
}

async fn upload_poster(server: &TestServer, path: &str) {
    let response = server
        .admin(Method::POST, &format!("{path}/thumbnails"))
        .multipart(file(png(8, 8)))
        .send()
        .await
        .expect("Upload");
    assert_eq!(response.status(), StatusCode::OK);
}

async fn status(
    server: &TestServer,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> StatusCode {
    send(server, None, method, path, body).await.status()
}

/// `(read lists, collections, read list posters, collection posters)` stored.
async fn counts(pool: &SqlitePool) -> [i64; 4] {
    let row: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM READLIST), (SELECT COUNT(*) FROM COLLECTION), \
         (SELECT COUNT(*) FROM THUMBNAIL_READLIST), (SELECT COUNT(*) FROM THUMBNAIL_COLLECTION)",
    )
    .fetch_one(pool)
    .await
    .expect("Counts");
    [row.0, row.1, row.2, row.3]
}

/// Scans library `B`, or empties its trash, and waits for the task.
async fn run(server: &TestServer, directory: &std::path::Path, library: &str, action: &str) {
    let path = format!("/api/v1/libraries/{library}/{action}");
    assert_eq!(
        status(server, Method::POST, &path, None).await,
        StatusCode::ACCEPTED
    );
    wait_for_scan_work(directory).await;
    wait_for_tasks(directory, |rows| {
        !rows.iter().any(|row| row.2.ends_with("$EmptyTrash"))
    })
    .await;
}

#[tokio::test]
async fn empty_sets_are_deleted_with_their_posters_only_when_enabled() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    let library_b: String = sqlx::query_scalar("SELECT ID FROM LIBRARY WHERE NAME = 'B'")
        .fetch_one(&f.pool)
        .await
        .expect("Library B");
    let list_a = create(&f.server, READ_LISTS, "bookIds", "A", &f.a1).await;
    let list_b = create(&f.server, READ_LISTS, "bookIds", "B", &f.b1).await;
    let collection_a = create(&f.server, COLLECTIONS, "seriesIds", "A", &f.series_a).await;
    let collection_b = create(&f.server, COLLECTIONS, "seriesIds", "B", &f.series_b).await;
    for path in [&list_a, &list_b, &collection_a] {
        upload_poster(&f.server, path).await;
    }

    // New databases enable both; disabled, the emptied sets stay after the scan and the trash
    // emptying, and an empty collection without a poster shows a black mosaic.
    update_settings(
        &f.server,
        json!({"deleteEmptyCollections": false, "deleteEmptyReadLists": false}),
    )
    .await;
    // Another series keeps library B from scanning empty, which deletes nothing.
    let other = directory.path().join("b/SC");
    fs::create_dir(&other).expect("Other series");
    fs::write(other.join("c1.cbz"), b"book").expect("Other book");
    fs::remove_file(directory.path().join("b/SB/b1.cbz")).expect("Remove the book");
    run(&f.server, directory.path(), &library_b, "scan").await;
    run(&f.server, directory.path(), &library_b, "empty-trash").await;
    assert_eq!(counts(&f.pool).await, [2, 2, 2, 1]);
    assert_black_poster(&f.server, &collection_b).await;

    // Each setting applies on its own, after trash emptying and after scans.
    update_settings(&f.server, json!({"deleteEmptyCollections": true})).await;
    run(&f.server, directory.path(), &library_b, "empty-trash").await;
    assert_eq!(counts(&f.pool).await, [2, 1, 2, 1]);
    update_settings(&f.server, json!({"deleteEmptyReadLists": true})).await;
    run(&f.server, directory.path(), &library_b, "scan").await;
    assert_eq!(
        counts(&f.pool).await,
        [1, 1, 1, 1],
        "the posters of list B go too"
    );
    for (path, expected) in [
        (&list_b, StatusCode::NOT_FOUND),
        (&collection_b, StatusCode::NOT_FOUND),
        (&list_a, StatusCode::OK),
        (&collection_a, StatusCode::OK),
    ] {
        assert_eq!(
            status(&f.server, Method::GET, path, None).await,
            expected,
            "{path}"
        );
    }
    // The cleanup leaves the remaining selections alone.
    let posters = list_at(&f.server, &format!("{list_a}/thumbnails")).await;
    assert_eq!(selected_ids(&posters).len(), 1);
    wait_for_index(&f.pool).await;
    f.server.stop().await;
    // The deleted sets leave the search index.
    for (entity, path) in [
        (SearchEntity::ReadList, &list_a),
        (SearchEntity::Collection, &collection_a),
    ] {
        let id = path.rsplit('/').next().expect("Set ID");
        assert_eq!(hits(directory.path(), entity, "*:*"), [id], "{path}");
    }
}
