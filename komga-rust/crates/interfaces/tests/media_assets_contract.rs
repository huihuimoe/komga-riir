use komga_application::media_assets::{
    CollectionThumbnailRecord, EntityThumbnailRecord, ReadlistThumbnailRecord,
    SeriesThumbnailRecord,
};
use komga_domain::media_assets::ThumbnailType;
use komga_interfaces::contracts::media_assets::{
    BookThumbnailDto, CollectionThumbnailDto, ReadListThumbnailDto, SeriesThumbnailDto,
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
