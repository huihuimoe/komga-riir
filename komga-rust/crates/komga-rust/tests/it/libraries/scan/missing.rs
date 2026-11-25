//! Content a library scan no longer finds: soft deleted after a complete listing, kept under
//! directories the scan could not list.

use std::fs;
use std::path::Path;

use serde_json::Value;

use super::super::create;
use super::{scan, scanned_library, set_directory_modified, touch};
use crate::support::TestServer;
use crate::support::db::{
    self,
    content::{self, books, deletion_dates, series, sidecars},
};
use crate::support::tasks::{TaskRow, clear_after_scans, wait_for_scan_work};

/// Rescans and returns the queued tasks, asserting the scan succeeded.
async fn rescan(server: &TestServer, directory: &Path, library: &Value) -> Vec<TaskRow> {
    clear_after_scans(server, directory).await;
    scan(server, library, "").await;
    let tasks = wait_for_scan_work(directory).await;
    assert!(
        tasks.iter().all(|row| !row.0.starts_with("SCAN_LIBRARY_")),
        "the scan succeeds: {tasks:?}"
    );
    tasks
}

#[tokio::test]
async fn missing_books_and_series_are_soft_deleted() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool, _) = scanned_library(directory.path()).await;
    let root = directory.path().join("comics");
    let series_a = root.join("Series A");
    let before = books(&pool).await;
    let series_ids: Vec<String> = series(&pool).await.into_iter().map(|row| row.0).collect();

    // Swap a book for another but keep the directory time: the lost book alone marks the series
    // changed, so a plain scan still picks up the new one. Another size leaves the new book
    // unmatched by the hashed lost one.
    let modified = fs::metadata(&series_a)
        .and_then(|metadata| metadata.modified())
        .expect("Series directory time");
    fs::remove_file(series_a.join("a 2.cbz")).expect("Remove book");
    fs::write(series_a.join("a 3.cbz"), b"another book").expect("New book");
    set_directory_modified(&series_a, modified);
    // The root keeps no book, so its series goes.
    fs::remove_file(root.join("one.pdf")).expect("Remove root book");
    let aggregations = content::aggregation_dates(&pool).await;
    rescan(&server, directory.path(), &library).await;

    let id = |name: &str| -> String {
        before
            .iter()
            .find(|row| row.1 == name)
            .map(|row| row.0.clone())
            .expect("Stored book")
    };
    let after = books(&pool).await;
    let new_id = after
        .iter()
        .find(|row| row.1 == "a 3")
        .map(|row| row.0.clone())
        .expect("New book added");
    // Numbering keeps soft-deleted books, as Kotlin `sortBooks` does.
    assert_eq!(
        after,
        [
            (id("a 1"), "a 1".to_owned(), 1, false),
            (id("a 10"), "a 10".to_owned(), 4, false),
            (id("a 2"), "a 2".to_owned(), 2, true),
            (new_id, "a 3".to_owned(), 3, false),
            (id("one"), "one".to_owned(), 1, true),
        ]
    );
    assert_eq!(
        series(&pool).await,
        [
            (series_ids[0].clone(), "Series A".to_owned(), 4, false),
            (series_ids[1].clone(), "comics".to_owned(), 1, true),
        ]
    );
    // The changed series alone is refreshed, which ends with the aggregation of its books.
    assert_eq!(
        content::aggregated_since(&pool, &aggregations).await,
        [series_ids[0].as_str()]
    );

    // Deleted content stays deleted, with its dates, on later scans.
    let dates = deletion_dates(&pool).await;
    rescan(&server, directory.path(), &library).await;
    assert_eq!(books(&pool).await, after);
    assert_eq!(deletion_dates(&pool).await, dates);
    server.stop().await;
}

#[tokio::test]
async fn directories_the_scan_cannot_list_keep_their_content() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let server = TestServer::start_claimed(directory.path()).await;
    let root = directory.path().join("comics");
    for file in [
        "Open/a.cbz",
        "Locked/b.cbz",
        "Locked/cover.jpg",
        "Unsearchable/c.cbz",
        "Unsearchable/Sub/d.cbz",
    ] {
        touch(&root.join(file));
    }
    let library = create(&server, "Comics", &root).await;
    let pool = db::main_pool(directory.path()).await;
    wait_for_scan_work(directory.path()).await;
    let stored = (books(&pool).await, series(&pool).await);
    assert_eq!(stored.0.len(), 4);
    let stored_sidecars = sidecars(&pool).await;
    assert_eq!(stored_sidecars.len(), 1);

    let unlock = Undo(Some(lock(&root)));
    let tasks = rescan(&server, directory.path(), &library).await;
    drop(unlock);
    assert_eq!((books(&pool).await, series(&pool).await), stored);
    assert_eq!(sidecars(&pool).await, stored_sidecars);
    assert!(
        tasks
            .iter()
            .all(|row| !row.0.starts_with("REFRESH_SERIES_METADATA_")),
        "unread content is left as it was: {tasks:?}"
    );

    rescan(&server, directory.path(), &library).await;
    assert_eq!((books(&pool).await, series(&pool).await), stored);
    assert_eq!(sidecars(&pool).await, stored_sidecars);
    server.stop().await;
}

/// Undoes when dropped, so that a failing test still leaves a directory that can be removed.
struct Undo<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> Drop for Undo<F> {
    fn drop(&mut self) {
        if let Some(undo) = self.0.take() {
            undo();
        }
    }
}

/// Makes `Locked` unlistable and `Unsearchable` listable with unreadable entries; returns what
/// undoes it.
#[cfg(unix)]
fn lock(root: &Path) -> impl FnOnce() {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let set = |name: &str, mode: u32| {
        fs::set_permissions(root.join(name), Permissions::from_mode(mode)).expect("Permissions");
    };
    set("Locked", 0o000);
    set("Unsearchable", 0o600);
    move || {
        set("Locked", 0o755);
        set("Unsearchable", 0o755);
    }
}

/// Makes `Locked` unlistable; returns what undoes it. Windows has no listable directory with
/// unreadable entries: std reads the metadata of an entry it cannot open from the listing.
#[cfg(windows)]
fn lock(root: &Path) -> impl FnOnce() {
    // Everyone; a deny entry binds administrators too.
    const EVERYONE: &str = "*S-1-1-0";
    let locked = root.join("Locked");
    icacls(&locked, &["/deny", &format!("{EVERYONE}:(RD)")]);
    move || icacls(&locked, &["/remove:d", EVERYONE])
}

#[cfg(windows)]
fn icacls(path: &Path, args: &[&str]) {
    use std::process::{Command, Stdio};

    let status = Command::new("icacls")
        .arg(path)
        .args(args)
        .stdout(Stdio::null())
        .status()
        .expect("Run icacls");
    assert!(
        status.success(),
        "icacls {args:?} on {}: {status}",
        path.display()
    );
}
