use komga_interfaces::contracts::common::{KotlinLocalDate, KotlinUtcDateTime, PageDto};
use serde_json::json;

#[test]
fn kotlin_utc_datetime_accepts_known_historical_storage_formats() {
    let values = [
        "2024-01-01 00:00:00",
        "2024-01-01T00:00:00Z",
        "2024-01-01T00:00:00.123Z",
        "1704067200",
    ];

    for value in values {
        let wire = KotlinUtcDateTime::parse(value)
            .unwrap_or_else(|error| panic!("known datetime format should parse: {value}: {error}"));
        let serialized = serde_json::to_value(wire).expect("datetime should serialize");
        assert!(
            serialized
                .as_str()
                .is_some_and(|value| value.ends_with('Z'))
        );
    }
}

#[test]
fn kotlin_utc_datetime_rejects_unknown_storage_values() {
    assert!(KotlinUtcDateTime::parse("not-a-date").is_err());
}

#[test]
fn kotlin_local_date_serializes_with_kotlin_format() {
    let date = KotlinLocalDate::parse("2024-01-15").expect("local date should parse");

    assert_eq!(
        serde_json::to_value(date).expect("date should serialize"),
        json!("2024-01-15")
    );
    assert!(KotlinLocalDate::parse("2024-15-01").is_err());
}

#[test]
fn paged_dto_exposes_spring_shape_without_json_mutation() {
    let payload = PageDto::paged(vec![json!({ "id": "book-1" })], 2, 20, 41, 3, true);

    assert_eq!(
        serde_json::to_value(payload).expect("page should serialize"),
        json!({
            "content": [{ "id": "book-1" }],
            "pageable": {
                "pageNumber": 2,
                "pageSize": 20,
                "sort": { "empty": false, "sorted": true, "unsorted": false },
                "offset": 40,
                "paged": true,
                "unpaged": false
            },
            "last": true,
            "totalElements": 41,
            "totalPages": 3,
            "first": false,
            "size": 20,
            "number": 2,
            "sort": { "empty": false, "sorted": true, "unsorted": false },
            "numberOfElements": 1,
            "empty": false
        })
    );
}

#[test]
fn unpaged_dto_has_explicit_unpaged_semantics() {
    let payload = PageDto::unpaged(vec![json!({ "id": "book-1" })], true);
    let serialized = serde_json::to_value(payload).expect("page should serialize");

    assert_eq!(serialized["pageable"]["paged"], json!(false));
    assert_eq!(serialized["pageable"]["unpaged"], json!(true));
    assert_eq!(serialized["pageable"]["offset"], json!(0));
    assert_eq!(serialized["totalPages"], json!(1));
}

#[test]
fn page_dto_can_preserve_unpaged_totals_from_a_read_model_page() {
    let payload = PageDto::from_parts(Vec::<serde_json::Value>::new(), 2, 20, 5, 1, false, true);
    let serialized = serde_json::to_value(payload).expect("page should serialize");

    assert_eq!(serialized["pageable"]["pageSize"], json!(20));
    assert_eq!(serialized["pageable"]["offset"], json!(0));
    assert_eq!(serialized["pageable"]["paged"], json!(false));
    assert_eq!(serialized["totalElements"], json!(5));
    assert_eq!(serialized["totalPages"], json!(1));
}
