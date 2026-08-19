use komga_interfaces::contracts::common::KotlinLocalDateTime;
use komga_interfaces::contracts::transient_books::{TransientBookDto, TransientBookPageDto};
use serde_json::json;

#[test]
fn transient_book_dto_matches_kotlin_field_shape() {
    let payload = serde_json::to_value(TransientBookDto {
        id: "transient-1".to_string(),
        name: "Book.cbz".to_string(),
        url: "/library/Book.cbz".to_string(),
        file_last_modified: KotlinLocalDateTime::from_unix_timestamp_nanos(123_456_789)
            .expect("local datetime should be valid"),
        size_bytes: 1024,
        size: "1 KiB".to_string(),
        status: "READY".to_string(),
        media_type: "image/png".to_string(),
        pages: vec![TransientBookPageDto {
            number: 1,
            file_name: "page.png".to_string(),
            media_type: "image/png".to_string(),
            width: Some(100),
            height: Some(200),
            size_bytes: None,
            size: String::new(),
        }],
        files: vec!["page.png".to_string()],
        comment: String::new(),
        number: None,
        series_id: None,
    })
    .expect("transient book should serialize");

    assert_eq!(payload["id"], json!("transient-1"));
    assert!(
        payload["fileLastModified"]
            .as_str()
            .is_some_and(|value| value.contains(".123456789"))
    );
    assert_eq!(
        payload["pages"][0],
        json!({
            "number": 1,
            "fileName": "page.png",
            "mediaType": "image/png",
            "width": 100,
            "height": 200,
            "sizeBytes": null,
            "size": ""
        })
    );
    assert!(payload["number"].is_null());
    assert!(payload["seriesId"].is_null());
}

#[test]
fn kotlin_local_datetime_rejects_out_of_range_timestamps() {
    assert!(KotlinLocalDateTime::from_unix_timestamp_nanos(i128::MAX).is_err());
}
