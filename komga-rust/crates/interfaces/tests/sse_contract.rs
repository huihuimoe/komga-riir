use komga_interfaces::contracts::sse::{BookImportSseDto, TaskQueueSseDto};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn sse_dtos_match_kotlin_field_names_and_nullable_message() {
    let payload = serde_json::to_value(BookImportSseDto {
        book_id: None,
        source_file: "source.cbz".to_string(),
        success: false,
        message: None,
    })
    .expect("book import event should serialize");

    assert_eq!(
        payload,
        json!({
            "bookId": null,
            "sourceFile": "source.cbz",
            "success": false,
            "message": null,
        })
    );
}

#[test]
fn task_queue_sse_dto_matches_kotlin_field_names() {
    let payload = serde_json::to_value(TaskQueueSseDto {
        count: 2,
        count_by_type: BTreeMap::from([(String::from("scanLibrary"), 2)]),
        error: None,
    })
    .expect("task queue event should serialize");

    assert_eq!(
        payload,
        json!({ "count": 2, "countByType": { "scanLibrary": 2 } })
    );
}
