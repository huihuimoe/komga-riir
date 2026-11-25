//! Sync point snapshots and the changes between two of them (`SyncPointLifecycle`).

use std::path::Path;

use komga_db::{Database, KomgaUserDao};
use komga_service::SyncPointService;
use komga_types::{
    ApiKeyId, KomgaUser, SyncAnchors, SyncPointBooks, SyncPointId, SyncPointReadLists,
};
use reqwest::{Method, StatusCode};
use serde_json::json;
use sqlx::SqlitePool;

use crate::books::epub::divina_book;
use crate::books::read_progress::mark;
use crate::books::reading::access::create_reader;
use crate::books::reading::fetch;
use crate::books::{PNG, library_with_books};
use crate::support::ADMIN_EMAIL;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;

async fn execute(pool: &SqlitePool, sql: &'static str, book_id: &str) {
    sqlx::query(sql)
        .bind(book_id)
        .execute(pool)
        .await
        .expect("Book change");
}

async fn take(
    service: &SyncPointService,
    to: &SyncPointId,
    books: SyncPointBooks<'_>,
    size: u32,
) -> (Vec<String>, u64) {
    let (items, total) = service
        .take_books(to, books, size)
        .await
        .expect("Taken books");
    let ids = items.iter().map(|book| book.book_id.to_string()).collect();
    (ids, total)
}

async fn database(directory: &Path) -> Database {
    Database::connect(&directory.join("database.sqlite"))
        .await
        .expect("Server database")
}

async fn user(directory: &Path, email: &str) -> KomgaUser {
    KomgaUserDao::find_by_email_ignore_case(&database(directory).await, email)
        .await
        .expect("User lookup")
        .expect("User")
}

async fn service(directory: &Path) -> (SyncPointService, KomgaUser) {
    let service = SyncPointService::new(database(directory).await);
    (service, user(directory, ADMIN_EMAIL).await)
}

async fn sync_point_keys(pool: &SqlitePool) -> Vec<Option<String>> {
    sqlx::query_scalar("SELECT API_KEY_ID FROM SYNC_POINT ORDER BY API_KEY_ID")
        .fetch_all(pool)
        .await
        .expect("Sync points")
}

#[tokio::test]
async fn books_are_handed_out_once_across_pages_with_the_on_deck_read_list() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let epub = divina_book(&cbz::fixture(PNG));
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/a.epub", epub.clone()),
            ("Series/b.epub", epub.clone()),
            ("Series/comic.cbz", cbz::zip(&[("p.png", STORED, b"png")])),
            ("Done/x1.epub", epub.clone()),
            ("Done/x2.epub", epub),
        ],
    )
    .await;
    let x1 = book(&pool, "x1").await.0;
    let x2 = book(&pool, "x2").await.0;
    mark(&server, &x1, json!({"completed": true})).await;
    let (service, admin) = service(directory.path()).await;
    let point = service.create(&admin, None).await.expect("Sync point");

    let mut taken = Vec::new();
    for (expected, total) in [(2, 4), (2, 2), (0, 0)] {
        let (ids, before) = take(&service, &point.id, SyncPointBooks::All, 2).await;
        assert_eq!((ids.len(), before), (expected, total));
        taken.extend(ids);
    }
    let mut expected = Vec::new();
    for name in ["a", "b", "x1", "x2"] {
        expected.push(book(&pool, name).await.0);
    }
    expected.sort();
    assert_eq!(taken, expected, "the EPUB books, each once, by ID");

    let (read_lists, total) = service
        .take_read_lists(&point.id, SyncPointReadLists::All, 10)
        .await
        .expect("Read lists");
    assert_eq!(total, 1);
    assert_eq!(read_lists[0].read_list_id, "KOMGA-ONDECK");
    assert_eq!(read_lists[0].name, "On Deck");
    let on_deck: Vec<String> =
        sqlx::query_scalar("SELECT BOOK_ID FROM SYNC_POINT_READLIST_BOOK WHERE SYNC_POINT_ID = ?")
            .bind(point.id.to_string())
            .fetch_all(&pool)
            .await
            .expect("On deck books");
    assert_eq!(on_deck, [x2]);
}

#[tokio::test]
async fn changes_since_an_earlier_sync_point_are_told_apart() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let epub = divina_book(&cbz::fixture(PNG));
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/progress.epub", epub.clone()),
            ("Series/metadata.epub", epub.clone()),
            ("Series/added.epub", epub.clone()),
            ("Series/removed.epub", epub.clone()),
            ("Series/same.epub", epub),
        ],
    )
    .await;
    let [progress, metadata, added, removed, same] = {
        let mut ids = Vec::new();
        for name in ["progress", "metadata", "added", "removed", "same"] {
            ids.push(book(&pool, name).await.0);
        }
        <[String; 5]>::try_from(ids).expect("Five books")
    };
    // Only the selected thumbnail is part of the snapshot.
    let other_thumbnail = r#"
        INSERT INTO THUMBNAIL_BOOK (ID, BOOK_ID, URL, THUMBNAIL, SELECTED, TYPE)
        SELECT ID || '-other', BOOK_ID, URL, THUMBNAIL, 0, TYPE FROM THUMBNAIL_BOOK
        WHERE BOOK_ID = ?
    "#;
    execute(&pool, other_thumbnail, &same).await;
    // Kotlin misses progress on books without a thumbnail, and takes a first hash for a change.
    execute(
        &pool,
        "DELETE FROM THUMBNAIL_BOOK WHERE BOOK_ID = ?",
        &progress,
    )
    .await;
    execute(
        &pool,
        "UPDATE BOOK SET FILE_HASH = '' WHERE ID = ?",
        &progress,
    )
    .await;
    execute(
        &pool,
        "UPDATE MEDIA SET STATUS = 'ERROR' WHERE BOOK_ID = ?",
        &added,
    )
    .await;
    let (service, admin) = service(directory.path()).await;
    let from = service.create(&admin, None).await.expect("Sync point").id;

    mark(&server, &progress, json!({"page": 1})).await;
    execute(
        &pool,
        "UPDATE BOOK SET FILE_HASH = 'hash' WHERE ID = ?",
        &progress,
    )
    .await;
    let later = r#"
UPDATE BOOK_METADATA SET LAST_MODIFIED_DATE = datetime(LAST_MODIFIED_DATE, '+1 day')
WHERE BOOK_ID = ?
"#;
    execute(&pool, later, &metadata).await;
    execute(
        &pool,
        "UPDATE MEDIA SET STATUS = 'READY' WHERE BOOK_ID = ?",
        &added,
    )
    .await;
    let deleted = "UPDATE BOOK SET DELETED_DATE = CURRENT_TIMESTAMP WHERE ID = ?";
    execute(&pool, deleted, &removed).await;
    let to = service.create(&admin, None).await.expect("Sync point").id;

    let cases = [
        (SyncPointBooks::AddedSince(&from), added),
        (SyncPointBooks::ChangedSince(&from), metadata),
        (SyncPointBooks::ReadProgressChangedSince(&from), progress),
        (SyncPointBooks::RemovedSince(&from), removed),
    ];
    for (books, expected) in cases {
        assert_eq!(take(&service, &to, books, 10).await, (vec![expected], 1));
        assert_eq!(
            take(&service, &to, books, 10).await,
            (vec![], 0),
            "taken once"
        );
    }
}

#[tokio::test]
async fn an_interrupted_sync_resumes_and_the_finished_one_becomes_the_baseline() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let epub = divina_book(&cbz::fixture(PNG));
    let (_server, _, pool) = library_with_books(
        directory.path(),
        &[("Series/a.epub", epub.clone()), ("Series/b.epub", epub)],
    )
    .await;
    let (service, admin) = service(directory.path()).await;
    let round = service
        .start(&admin, None, &SyncAnchors::default())
        .await
        .expect("First round");
    assert!(round.from.is_none(), "the first sync is a full one");
    let (first, _) = take(&service, &round.to.id, SyncPointBooks::All, 1).await;
    let anchors = service.finish(round, true).await.expect("Paused");

    // The next page, or a retry of a lost response, carries the same anchors.
    let round = service
        .start(&admin, None, &anchors)
        .await
        .expect("Resumed");
    assert_eq!(Some(&round.to.id), anchors.ongoing.as_ref());
    let (rest, total) = take(&service, &round.to.id, SyncPointBooks::All, 10).await;
    assert_eq!(total, 1);
    assert_ne!(rest, first);
    let synced = service.finish(round, false).await.expect("Synced");
    assert_eq!(synced.ongoing, None);
    assert_eq!(synced.last_synced, anchors.ongoing);

    // A paused sync keeps its baseline.
    let round = service
        .start(&admin, None, &synced)
        .await
        .expect("Next sync");
    let paused = service.finish(round, true).await.expect("Next paused");
    assert_eq!(paused.last_synced, synced.last_synced);
    let round = service
        .start(&admin, None, &paused)
        .await
        .expect("Next resumed");
    let from = round.from.as_ref().expect("Baseline").id.clone();
    assert_eq!(Some(&from), synced.last_synced.as_ref());
    let added = SyncPointBooks::AddedSince(&from);
    assert_eq!(take(&service, &round.to.id, added, 10).await, (vec![], 0));
    let next = service.finish(round, false).await.expect("Next synced");
    let remaining: Vec<String> = sqlx::query_scalar("SELECT ID FROM SYNC_POINT")
        .fetch_all(&pool)
        .await
        .expect("Sync points");
    let baseline = next.last_synced.expect("New baseline").to_string();
    assert_eq!(remaining, [baseline], "the replaced baseline is deleted");
}

#[tokio::test]
async fn missing_or_foreign_anchors_start_a_full_sync() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let epub = divina_book(&cbz::fixture(PNG));
    let (server, _, _pool) = library_with_books(directory.path(), &[("Series/a.epub", epub)]).await;
    create_reader(&server, "other@example.com", json!({"roles": []})).await;
    let (service, admin) = service(directory.path()).await;
    let other = user(directory.path(), "other@example.com").await;
    let foreign = service.create(&other, None).await.expect("Other sync").id;
    let anchors = SyncAnchors {
        ongoing: Some(foreign.clone()),
        last_synced: Some(SyncPointId::from("deleted".to_owned())),
    };

    let round = service.start(&admin, None, &anchors).await.expect("Round");
    assert_ne!(round.to.id, foreign);
    assert_eq!(round.to.user_id, admin.id);
    assert!(round.from.is_none());
    assert_eq!(
        take(&service, &round.to.id, SyncPointBooks::All, 10)
            .await
            .1,
        1
    );
}

#[tokio::test]
async fn books_of_a_library_no_longer_shared_are_removed() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let epub = divina_book(&cbz::fixture(PNG));
    let books = [("Series/a.epub", epub.clone()), ("Series/b.epub", epub)];
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let a = book(&pool, "a").await.0;
    let mut ids = vec![a.clone(), book(&pool, "b").await.0];
    ids.sort();
    let email = "reader@example.com";
    let access = create_reader(&server, email, json!({"roles": []})).await;
    let path = format!("/api/v1/books/{a}/read-progress");
    let request = access.request(&server, Method::PATCH, &path);
    let response = fetch(request.json(&json!({"completed": true}))).await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    let (service, _) = service(directory.path()).await;
    let reader = user(directory.path(), email).await;
    let round = service
        .start(&reader, None, &SyncAnchors::default())
        .await
        .expect("First round");
    assert_eq!(
        take(&service, &round.to.id, SyncPointBooks::All, 10)
            .await
            .0,
        ids
    );
    let anchors = service.finish(round, false).await.expect("Synced");

    let response = server
        .admin(Method::PATCH, &format!("/api/v2/users/{}", reader.id))
        .json(&json!({"sharedLibraries": {"all": false, "libraryIds": []}}))
        .send()
        .await
        .expect("Unshare");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let reader = user(directory.path(), email).await;
    let round = service.start(&reader, None, &anchors).await.expect("Round");
    let from = round.from.as_ref().expect("Baseline").id.clone();
    let removed = SyncPointBooks::RemovedSince(&from);
    assert_eq!(take(&service, &round.to.id, removed, 10).await, (ids, 2));
    // On Deck books come only from the libraries the user can see, unlike Kotlin.
    let (read_lists, _) = service
        .take_read_lists(&round.to.id, SyncPointReadLists::RemovedSince(&from), 10)
        .await
        .expect("Read lists");
    assert_eq!(read_lists[0].read_list_id, "KOMGA-ONDECK");
}

#[tokio::test]
async fn users_delete_their_sync_points_by_api_key_or_all() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let epub = divina_book(&cbz::fixture(PNG));
    let (server, _, pool) = library_with_books(directory.path(), &[("Series/a.epub", epub)]).await;
    let other = create_reader(&server, "other@example.com", json!({"roles": []})).await;
    let (service, admin) = service(directory.path()).await;
    for key in [None, Some("k1"), Some("k2"), Some("k3")] {
        let key = key.map(ApiKeyId::from);
        service
            .create(&admin, key.as_ref())
            .await
            .expect("Sync point");
    }
    let other_user = user(directory.path(), "other@example.com").await;
    let kept = ApiKeyId::from("k1");
    service
        .create(&other_user, Some(&kept))
        .await
        .expect("Other");

    let delete = |path: &str| server.admin(Method::DELETE, path).send();
    let response = delete("/api/v1/syncpoints/me?key_id=k1,k2")
        .await
        .expect("Delete");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let k = |key: &str| Some(key.to_owned());
    assert_eq!(sync_point_keys(&pool).await, [None, k("k1"), k("k3")]);
    let response = delete("/api/v1/syncpoints/me").await.expect("Delete all");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        sync_point_keys(&pool).await,
        [k("k1")],
        "other users keep theirs"
    );
    let response = other
        .request(&server, Method::DELETE, "/api/v1/syncpoints/me?key_id=k1")
        .send()
        .await
        .expect("Reader delete");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(sync_point_keys(&pool).await.is_empty());
}
