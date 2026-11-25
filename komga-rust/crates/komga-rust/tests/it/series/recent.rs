//! Recent series (`SeriesController.getSeriesLatest/getSeriesNew/getSeriesUpdated`).

use std::fs;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::list::names;
use crate::books::latest::rescan;
use crate::books::library_with_books;
use crate::books::reading::pages_book;
use crate::read_lists::{create_user, rate, send};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

async fn get(server: &TestServer, email: Option<&str>, path: &str) -> Value {
    let response = send(server, email, Method::GET, path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    openapi::checked(Method::GET, response).await
}

/// Series `Alpha`, `Beta` and `Gamma` scanned together; `Delta` is added later and `Alpha` gets
/// a book, which modifies it. Creation dates have a precision of seconds, so the initial batch is
/// aged directly instead of sleeping a second.
async fn recent_library(directory: &std::path::Path) -> (TestServer, Value) {
    let books = ["Alpha/a1.cbz", "Beta/b1.cbz", "Gamma/g1.cbz"].map(|path| (path, pages_book()));
    let (server, library, pool) = library_with_books(directory, &books).await;
    sqlx::query(
        "UPDATE SERIES SET CREATED_DATE = datetime(CREATED_DATE, '-10 seconds'), \
                           LAST_MODIFIED_DATE = datetime(LAST_MODIFIED_DATE, '-10 seconds')",
    )
    .execute(&pool)
    .await
    .expect("Age initial series");
    pool.close().await;
    let root = directory.join("comics");
    fs::create_dir_all(root.join("Delta")).expect("Series directory");
    fs::write(root.join("Delta/d1.cbz"), pages_book()).expect("Added series");
    fs::write(root.join("Alpha/a2.cbz"), pages_book()).expect("Added book");
    rescan(&server, directory, &library, "").await;
    (server, library)
}

#[tokio::test]
async fn recent_series_come_newest_first() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library) = recent_library(directory.path()).await;

    let latest = get(&server, None, "/api/v1/series/latest").await;
    assert_eq!(names(&latest)[..2], ["Alpha", "Delta"]);
    assert_eq!(latest["totalElements"], 4);
    assert_eq!(latest["sort"]["sorted"], true);
    let new = get(&server, None, "/api/v1/series/new?sort=name").await;
    assert_eq!(names(&new)[0], "Delta", "sort parameters do not apply");
    let updated = get(&server, None, "/api/v1/series/updated").await;
    assert_eq!(names(&updated), ["Alpha"]);
    assert_eq!(updated["totalElements"], 1);
    let second = get(&server, None, "/api/v1/series/latest?size=1&page=1").await;
    assert_eq!(names(&second), ["Delta"]);
    assert_eq!(second["totalElements"], 4);
    let unpaged = get(&server, None, "/api/v1/series/new?unpaged=true&size=1").await;
    assert_eq!(names(&unpaged).len(), 4);

    let library_id = library["id"].as_str().expect("Library ID");
    let path = format!("/api/v1/series/updated?library_id={library_id}");
    assert_eq!(names(&get(&server, None, &path).await), ["Alpha"]);
    let elsewhere = get(&server, None, "/api/v1/series/latest?library_id=0").await;
    assert_eq!(elsewhere["totalElements"], 0);
    let oneshots = get(&server, None, "/api/v1/series/new?oneshot=true").await;
    assert_eq!(oneshots["totalElements"], 0);
    assert_eq!(
        get(&server, None, "/api/v1/series/new?oneshot=false").await["totalElements"],
        4
    );

    fs::remove_dir_all(directory.path().join("comics/Beta")).expect("Remove a series");
    rescan(&server, directory.path(), &library, "").await;
    let deleted = get(&server, None, "/api/v1/series/latest?deleted=true").await;
    assert_eq!(names(&deleted), ["Beta"]);
    let kept = get(&server, None, "/api/v1/series/new?deleted=false").await;
    assert_eq!(kept["totalElements"], 3);
    server.stop().await;
}

#[tokio::test]
async fn users_see_only_recent_series_they_may_see() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _) = recent_library(directory.path()).await;
    let pool = crate::support::db::main_pool(directory.path()).await;
    rate(&server, &book(&pool, "d1").await.1, 18).await;
    let sixteen = json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    create_user(&server, "restricted@example.com", sixteen).await;
    let restricted = Some("restricted@example.com");

    let new = get(&server, restricted, "/api/v1/series/new").await;
    assert_eq!(new["totalElements"], 3);
    assert!(!names(&new).contains(&"Delta"));
    assert_eq!(
        new["content"][0]["url"], "",
        "only admins see directory paths"
    );
    rate(&server, &book(&pool, "a1").await.1, 18).await;
    let updated = get(&server, restricted, "/api/v1/series/updated").await;
    assert_eq!(updated["totalElements"], 0);

    let anonymous = server
        .request(Method::GET, "/api/v1/series/latest")
        .send()
        .await
        .expect("Anonymous request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    server.stop().await;
}
