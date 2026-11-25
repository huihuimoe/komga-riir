//! Series collections over real HTTP against scanned libraries.

mod crud;
mod list;
mod of_series;
mod series;
mod thumbnails;

use std::fs;
use std::path::Path;

use sqlx::SqlitePool;

use crate::libraries::create;
use crate::read_lists::{create_user, rate, send};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::tasks::wait_for_scan_work;
use crate::support::{TestServer, db};

const COLLECTIONS: &str = "/api/v1/collections";
const PNG: &str = "komga-image/tests/fixtures/photo.png";

/// A claimed server over library `A` (series `SA1`, `SA2`) and library `B` (series `SB`), with
/// the IDs of what it scanned.
struct Fixture {
    server: TestServer,
    pool: SqlitePool,
    library_a: String,
    a1: String,
    a2: String,
    b: String,
}

async fn fixture(directory: &Path) -> Fixture {
    let png = cbz::fixture(PNG);
    let file = cbz::zip(&[("page.png", STORED, &png)]);
    for path in ["a/SA1/a1.cbz", "a/SA2/a2.cbz", "b/SB/b.cbz"] {
        let path = directory.join(path);
        fs::create_dir_all(path.parent().expect("Series directory")).expect("Series directory");
        fs::write(path, &file).expect("Book file");
    }
    let server = TestServer::start_claimed(directory).await;
    let library_a = create(&server, "A", &directory.join("a")).await;
    create(&server, "B", &directory.join("b")).await;
    wait_for_scan_work(directory).await;
    let pool = db::main_pool(directory).await;
    let (_, a1) = book(&pool, "a1").await;
    let (_, a2) = book(&pool, "a2").await;
    let (_, b) = book(&pool, "b").await;
    Fixture {
        server,
        pool,
        library_a: library_a["id"].as_str().expect("Library ID").to_owned(),
        a1,
        a2,
        b,
    }
}

/// `(SERIES_COUNT, SERIES_ID by NUMBER)` of the collection `id`.
async fn stored_members(pool: &SqlitePool, id: &str) -> (i64, Vec<(i64, String)>) {
    let count = sqlx::query_scalar("SELECT SERIES_COUNT FROM COLLECTION WHERE ID = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("Collection count");
    let members = sqlx::query_as(
        "SELECT NUMBER, SERIES_ID FROM COLLECTION_SERIES WHERE COLLECTION_ID = ? ORDER BY NUMBER",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .expect("Collection members");
    (count, members)
}
