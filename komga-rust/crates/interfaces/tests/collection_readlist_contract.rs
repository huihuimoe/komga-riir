use komga_application::discovery::{CollectionReadModel, ReadListReadModel};
use komga_interfaces::contracts::discovery::{CollectionDto, ReadListDto};
use serde_json::json;

fn collection() -> CollectionReadModel {
    CollectionReadModel {
        id: "collection-1".to_string(),
        name: "Collection 1".to_string(),
        ordered: true,
        series_ids: vec!["series-1".to_string(), "series-2".to_string()],
        created_date: "2024-01-01 00:00:00".to_string(),
        last_modified_date: "2024-01-02T00:00:00Z".to_string(),
        filtered: false,
    }
}

fn readlist() -> ReadListReadModel {
    ReadListReadModel {
        id: "readlist-1".to_string(),
        name: "Read list 1".to_string(),
        summary: "Summary".to_string(),
        ordered: true,
        book_ids: vec!["book-1".to_string(), "book-2".to_string()],
        created_date: "2024-01-03 00:00:00".to_string(),
        last_modified_date: "2024-01-04T00:00:00Z".to_string(),
        filtered: true,
    }
}

#[test]
fn collection_dto_matches_kotlin_field_shape_and_formats() {
    let payload = serde_json::to_value(
        CollectionDto::from_read_model(&collection()).expect("collection should map"),
    )
    .expect("collection should serialize");

    assert_eq!(
        payload
            .as_object()
            .expect("collection should be an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "createdDate",
            "filtered",
            "id",
            "lastModifiedDate",
            "name",
            "ordered",
            "seriesIds",
        ]
    );
    assert_eq!(payload["createdDate"], json!("2024-01-01T00:00:00Z"));
    assert_eq!(payload["lastModifiedDate"], json!("2024-01-02T00:00:00Z"));
    assert_eq!(payload["seriesIds"], json!(["series-1", "series-2"]));
}

#[test]
fn readlist_dto_matches_kotlin_field_shape_and_formats() {
    let payload = serde_json::to_value(
        ReadListDto::from_read_model(&readlist()).expect("read list should map"),
    )
    .expect("read list should serialize");

    assert_eq!(
        payload
            .as_object()
            .expect("read list should be an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "bookIds",
            "createdDate",
            "filtered",
            "id",
            "lastModifiedDate",
            "name",
            "ordered",
            "summary",
        ]
    );
    assert_eq!(payload["createdDate"], json!("2024-01-03T00:00:00Z"));
    assert_eq!(payload["lastModifiedDate"], json!("2024-01-04T00:00:00Z"));
    assert_eq!(payload["bookIds"], json!(["book-1", "book-2"]));
}

#[test]
fn collection_dto_rejects_invalid_wire_dates() {
    let mut invalid = collection();
    invalid.created_date = "not-a-date".to_string();

    assert!(CollectionDto::from_read_model(&invalid).is_err());
}
