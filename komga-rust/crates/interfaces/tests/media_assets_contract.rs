use komga_application::media_assets::{
    BookPageRecord, CollectionThumbnailRecord, EntityThumbnailRecord, ReadlistThumbnailRecord,
    SeriesThumbnailRecord,
};
use komga_domain::media_assets::ThumbnailType;
use komga_interfaces::contracts::media_assets::{
    BookPageDto, BookProgressionDeviceDto, BookProgressionDto, BookThumbnailDto,
    CollectionThumbnailDto, ReadListThumbnailDto, SeriesThumbnailDto, TachiyomiReadListProgressDto,
    TachiyomiSeriesProgressDto,
};
use serde_json::json;

fn fields() -> (ThumbnailType, bool, String, i64, i64, i64) {
    (
        ThumbnailType::UserUploaded,
        true,
        "image/jpeg".to_string(),
        123,
        200,
        300,
    )
}

#[test]
fn thumbnail_dtos_preserve_kotlin_owner_field_names() {
    let (thumbnail_type, selected, media_type, file_size, width, height) = fields();
    let book = BookThumbnailDto::from_record(&EntityThumbnailRecord {
        id: "thumb-book".to_string(),
        book_id: "book-1".to_string(),
        thumbnail_type,
        selected,
        media_type: media_type.clone(),
        file_size,
        width,
        height,
    });
    let series = SeriesThumbnailDto::from_record(&SeriesThumbnailRecord {
        id: "thumb-series".to_string(),
        series_id: "series-1".to_string(),
        thumbnail_type,
        selected,
        media_type: media_type.clone(),
        file_size,
        width,
        height,
    });
    let readlist = ReadListThumbnailDto::from_record(&ReadlistThumbnailRecord {
        id: "thumb-readlist".to_string(),
        readlist_id: "readlist-1".to_string(),
        thumbnail_type,
        selected,
        media_type: media_type.clone(),
        file_size,
        width,
        height,
        thumbnail: vec![],
    });
    let collection = CollectionThumbnailDto::from_record(&CollectionThumbnailRecord {
        id: "thumb-collection".to_string(),
        collection_id: "collection-1".to_string(),
        thumbnail_type,
        selected,
        media_type,
        file_size,
        width,
        height,
        thumbnail: vec![],
    });

    assert_eq!(
        serde_json::to_value(book).expect("book thumbnail should serialize"),
        json!({
            "id": "thumb-book",
            "bookId": "book-1",
            "type": "USER_UPLOADED",
            "selected": true,
            "mediaType": "image/jpeg",
            "fileSize": 123,
            "width": 200,
            "height": 300,
        })
    );
    assert_eq!(
        serde_json::to_value(series).expect("series thumbnail should serialize")["seriesId"],
        json!("series-1")
    );
    assert_eq!(
        serde_json::to_value(readlist).expect("readlist thumbnail should serialize")["readListId"],
        json!("readlist-1")
    );
    assert_eq!(
        serde_json::to_value(collection).expect("collection thumbnail should serialize")["collectionId"],
        json!("collection-1")
    );
}

#[test]
fn progress_dtos_preserve_nested_and_tachiyomi_shapes() {
    let progression = BookProgressionDto {
        modified: "2024-01-01T00:00:00Z".to_string(),
        device: BookProgressionDeviceDto {
            id: "device-1".to_string(),
            name: "Reader".to_string(),
        },
        locator: json!({"href": "chapter.xhtml", "locations": {"progression": 0.5}}),
    };
    assert_eq!(
        serde_json::to_value(progression).expect("progression should serialize"),
        json!({
            "modified": "2024-01-01T00:00:00Z",
            "device": {"id": "device-1", "name": "Reader"},
            "locator": {"href": "chapter.xhtml", "locations": {"progression": 0.5}},
        })
    );

    let series = TachiyomiSeriesProgressDto {
        books_count: 2,
        books_read_count: 1,
        books_unread_count: 0,
        books_in_progress_count: 1,
        last_read_continuous_number_sort: 1.0,
        max_number_sort: 2.0,
    };
    assert_eq!(
        serde_json::to_value(series).expect("series progress should serialize"),
        json!({
            "booksCount": 2,
            "booksReadCount": 1,
            "booksUnreadCount": 0,
            "booksInProgressCount": 1,
            "lastReadContinuousNumberSort": 1.0,
            "maxNumberSort": 2.0,
        })
    );

    let readlist = TachiyomiReadListProgressDto {
        books_count: 2,
        books_read_count: 1,
        books_unread_count: 1,
        books_in_progress_count: 0,
        last_read_continuous_index: 1,
    };
    assert_eq!(
        serde_json::to_value(readlist).expect("readlist progress should serialize"),
        json!({
            "booksCount": 2,
            "booksReadCount": 1,
            "booksUnreadCount": 1,
            "booksInProgressCount": 0,
            "lastReadContinuousIndex": 1,
        })
    );
}

#[test]
fn book_page_dto_preserves_unknown_size_sentinel_shape() {
    let payload = serde_json::to_value(BookPageDto::from(BookPageRecord {
        number: 1,
        file_name: "page-1.jpg".to_string(),
        media_type: "image/jpeg".to_string(),
        width: Some(1200),
        height: Some(1800),
        file_size: -1,
    }))
    .expect("page should serialize");

    assert_eq!(
        payload,
        json!({
            "number": 1,
            "fileName": "page-1.jpg",
            "mediaType": "image/jpeg",
            "width": 1200,
            "height": 1800,
            "sizeBytes": null,
            "size": "",
        })
    );
}
