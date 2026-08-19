use std::collections::BTreeMap;

use komga_interfaces::contracts::common::KotlinLocalDateTime;
use komga_interfaces::contracts::history::HistoryEventDto;
use serde_json::json;

#[test]
fn history_event_dto_matches_kotlin_field_shape_and_local_datetime() {
    let payload = serde_json::to_value(HistoryEventDto {
        id: "event-1".to_string(),
        event_type: "BOOK_ADDED".to_string(),
        timestamp: KotlinLocalDateTime::parse("2024-01-02 03:04:05")
            .expect("history timestamp should parse"),
        book_id: None,
        series_id: Some("series-1".to_string()),
        properties: BTreeMap::from([(String::from("source"), String::from("scan"))]),
    })
    .expect("history event should serialize");

    assert_eq!(
        payload,
        json!({
            "id": "event-1",
            "type": "BOOK_ADDED",
            "timestamp": "2024-01-02T03:04:05",
            "bookId": null,
            "seriesId": "series-1",
            "properties": { "source": "scan" }
        })
    );
}
