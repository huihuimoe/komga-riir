//! The tasks database file a server under test writes.

use std::path::Path;
use std::time::Duration;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, SqliteConnection};
use tokio::time;

use super::TestServer;

/// `(ID, PRIORITY, CLASS, PAYLOAD, OWNER)` rows of the tasks database.
pub type TaskRow = (String, i32, String, String, Option<String>);

async fn connect(directory: &Path) -> SqliteConnection {
    let options = SqliteConnectOptions::new().filename(directory.join("tasks.sqlite"));
    SqliteConnection::connect_with(&options)
        .await
        .expect("Open tasks database")
}

pub async fn tasks(directory: &Path) -> Vec<TaskRow> {
    let mut connection = connect(directory).await;
    let rows = fetch_tasks(&mut connection).await;
    connection.close().await.expect("Close tasks database");
    rows
}

async fn fetch_tasks(connection: &mut SqliteConnection) -> Vec<TaskRow> {
    sqlx::query_as("SELECT ID, PRIORITY, CLASS, PAYLOAD, OWNER FROM TASK ORDER BY ID")
        .fetch_all(connection)
        .await
        .expect("Read tasks")
}

/// Queues the row Kotlin's `TasksDao` writes for `TaskEmitter.analyzeBook`.
pub async fn queue_kotlin_analysis(directory: &Path, book_id: &str, series_id: &str) {
    let id = format!("ANALYZE_BOOK_{book_id}");
    let payload = json!({"bookId": book_id, "priority": 4, "groupId": series_id, "uniqueId": id});
    queue_kotlin_task(directory, &id, 4, Some(series_id), "AnalyzeBook", &payload).await;
}

/// Queues the row Kotlin's `TasksDao` writes for `TaskEmitter.hashBookPages`.
pub async fn queue_kotlin_page_hashing(directory: &Path, book_id: &str) {
    let id = format!("HASH_BOOK_PAGES_{book_id}");
    let payload = json!({"bookId": book_id, "priority": 1, "groupId": null, "uniqueId": id});
    queue_kotlin_task(directory, &id, 1, None, "HashBookPages", &payload).await;
}

/// Queues the row Kotlin's `TasksDao` writes for `TaskEmitter.convertBookToCbz`.
pub async fn queue_kotlin_conversion(directory: &Path, book_id: &str, series_id: &str) {
    let id = format!("CONVERT_BOOK_{book_id}");
    let payload = json!({"bookId": book_id, "priority": 1, "groupId": series_id, "uniqueId": id});
    queue_kotlin_task(directory, &id, 1, Some(series_id), "ConvertBook", &payload).await;
}

/// Queues the row Kotlin's `TasksDao` writes for `TaskEmitter.repairExtensions`.
pub async fn queue_kotlin_repair(directory: &Path, book_id: &str, series_id: &str) {
    let id = format!("REPAIR_EXTENSION_{book_id}");
    let payload = json!({"bookId": book_id, "priority": 2, "groupId": series_id, "uniqueId": id});
    queue_kotlin_task(
        directory,
        &id,
        2,
        Some(series_id),
        "RepairExtension",
        &payload,
    )
    .await;
}

/// Queues the rows Kotlin's `SearchIndexController` writes for a Lucene index of version 5.
pub async fn queue_kotlin_index_upgrade(directory: &Path) {
    let rebuild = json!({"entities": ["Series"], "priority": 8, "groupId": null, "uniqueId": "REBUILD_INDEX"});
    queue_kotlin_task(
        directory,
        "REBUILD_INDEX",
        8,
        None,
        "RebuildIndex",
        &rebuild,
    )
    .await;
    let upgrade = json!({"priority": 8, "groupId": null, "uniqueId": "UPGRADE_INDEX"});
    queue_kotlin_task(
        directory,
        "UPGRADE_INDEX",
        8,
        None,
        "UpgradeIndex",
        &upgrade,
    )
    .await;
}

/// Queues a task of a class Kotlin removed in 1.21.3.
pub async fn queue_kotlin_removed_task(directory: &Path) {
    let id = "FIX_THUMBNAILS_WITHOUT_METADATA_2025-01-01T00:00:00";
    let payload = json!({"priority": 4, "groupId": "FixThumbnailsWithoutMetadata", "uniqueId": id});
    queue_kotlin_task(
        directory,
        id,
        4,
        Some("FixThumbnailsWithoutMetadata"),
        "FixThumbnailsWithoutMetadata",
        &payload,
    )
    .await;
}

async fn queue_kotlin_task(
    directory: &Path,
    id: &str,
    priority: i32,
    group: Option<&str>,
    simple_type: &str,
    payload: &Value,
) {
    let mut connection = connect(directory).await;
    sqlx::query(
        "INSERT INTO TASK (ID, PRIORITY, GROUP_ID, CLASS, SIMPLE_TYPE, PAYLOAD) \
         VALUES (?, ?, ?, 'org.gotson.komga.application.tasks.Task$' || ?, ?, ?)",
    )
    .bind(id)
    .bind(priority)
    .bind(group)
    .bind(simple_type)
    .bind(simple_type)
    .bind(payload.to_string())
    .execute(&mut connection)
    .await
    .expect("Kotlin task row");
    connection.close().await.expect("Close tasks database");
}

pub async fn wait_for_tasks(directory: &Path, done: impl Fn(&[TaskRow]) -> bool) -> Vec<TaskRow> {
    wait_for_tasks_within(directory, Duration::from_secs(10), done).await
}

pub async fn wait_for_tasks_within(
    directory: &Path,
    timeout: Duration,
    done: impl Fn(&[TaskRow]) -> bool,
) -> Vec<TaskRow> {
    let mut connection = connect(directory).await;
    let result = time::timeout(timeout, async {
        loop {
            let rows = fetch_tasks(&mut connection).await;
            if done(&rows) {
                return rows;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    connection.close().await.expect("Close tasks database");
    result.expect("Tasks reach the expected state")
}

/// Waits until no library scan, nor a book analysis, local artwork or metadata refresh, metadata
/// aggregation, file or page hashing or duplicate page removal a scan queues, is queued or
/// running; failed tasks stay queued.
pub async fn wait_for_scan_work(directory: &Path) -> Vec<TaskRow> {
    wait_for_scan_work_within(directory, Duration::from_secs(10)).await
}

pub async fn wait_for_scan_work_within(directory: &Path, timeout: Duration) -> Vec<TaskRow> {
    wait_for_tasks_within(directory, timeout, |rows| {
        rows.iter()
            .filter(|row| {
                [
                    "org.gotson.komga.application.tasks.Task$ScanLibrary",
                    "org.gotson.komga.application.tasks.Task$AnalyzeBook",
                    "org.gotson.komga.application.tasks.Task$RefreshBookLocalArtwork",
                    "org.gotson.komga.application.tasks.Task$RefreshSeriesLocalArtwork",
                    "org.gotson.komga.application.tasks.Task$RefreshBookMetadata",
                    "org.gotson.komga.application.tasks.Task$RefreshSeriesMetadata",
                    "org.gotson.komga.application.tasks.Task$AggregateSeriesMetadata",
                    "org.gotson.komga.application.tasks.Task$HashBook",
                    "org.gotson.komga.application.tasks.Task$HashBookKoreader",
                    "org.gotson.komga.application.tasks.Task$FindBooksWithMissingPageHash",
                    "org.gotson.komga.application.tasks.Task$HashBookPages",
                    "org.gotson.komga.application.tasks.Task$FindDuplicatePagesToDelete",
                    "org.gotson.komga.application.tasks.Task$RemoveHashedPages",
                ]
                .contains(&row.2.as_str())
            })
            .all(|row| row.3.contains("\"eraFailed\""))
    })
    .await
}

pub const GENERATE_BOOK_THUMBNAIL: &str =
    "org.gotson.komga.application.tasks.Task$GenerateBookThumbnail";

/// Waits until every queued book thumbnail generation has run or failed.
pub async fn wait_for_generations(directory: &Path) -> Vec<TaskRow> {
    wait_for_tasks(directory, |rows| {
        rows.iter()
            .filter(|row| row.2 == GENERATE_BOOK_THUMBNAIL)
            .all(|row| row.3.contains("\"eraFailed\""))
    })
    .await
}

/// Waits for the queued scans and analyses, then clears the queue through
/// `DELETE /api/v1/tasks`: they leave follow-ups that this build does not run yet, and queue tests
/// start from an empty queue.
pub async fn clear_after_scans(server: &TestServer, directory: &Path) {
    wait_for_scan_work(directory).await;
    let response = server
        .admin(Method::DELETE, "/api/v1/tasks")
        .send()
        .await
        .expect("Clear task queue");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(tasks(directory).await.is_empty(), "nothing else is running");
}
