use komga_application::discovery::{
    BookMetadataAuthorReadModel, BookMetadataLinkReadModel, BookReadModel,
    BookReadProgressReadModel,
};
use komga_domain::discovery::MediaStatus;
use komga_interfaces::contracts::discovery::BookDto;
use serde_json::json;

fn book() -> BookReadModel {
    BookReadModel {
        id: "book-1".to_string(),
        series_id: "series-1".to_string(),
        series_title: "Series".to_string(),
        series_title_sort: "Series".to_string(),
        library_id: "library-1".to_string(),
        name: "Book".to_string(),
        url: "file:/library%20root/books/book%201.cbz".to_string(),
        number: 1,
        created: "2024-01-01 00:00:00".to_string(),
        last_modified: "2024-01-02T00:00:00Z".to_string(),
        file_last_modified: "1704240000".to_string(),
        size_bytes: 1536,
        media_status: MediaStatus::Ready,
        media_type: "application/epub+zip".to_string(),
        media_pages_count: 5,
        media_comment: "ok".to_string(),
        media_epub_divina_compatible: true,
        media_epub_is_kepub: true,
        metadata_title: "Metadata title".to_string(),
        metadata_title_lock: true,
        metadata_summary: "Summary".to_string(),
        metadata_summary_lock: false,
        metadata_number: "1".to_string(),
        metadata_number_lock: true,
        metadata_number_sort: 1.0,
        metadata_number_sort_lock: false,
        metadata_release_date: Some("2024-01-15".to_string()),
        metadata_release_date_lock: true,
        metadata_authors: vec![BookMetadataAuthorReadModel {
            name: "Author".to_string(),
            role: "Writer".to_string(),
        }],
        metadata_authors_lock: false,
        metadata_tags: vec!["tag".to_string()],
        metadata_tags_lock: true,
        metadata_isbn: "isbn".to_string(),
        metadata_isbn_lock: false,
        metadata_links: vec![BookMetadataLinkReadModel {
            label: "Wiki".to_string(),
            url: "https://example.com".to_string(),
        }],
        metadata_links_lock: true,
        metadata_created: "2024-01-03 00:00:00".to_string(),
        metadata_last_modified: "2024-01-04T00:00:00Z".to_string(),
        read_progress: Some(BookReadProgressReadModel {
            page: 2,
            completed: false,
            read_date: None,
            created: "2024-01-05 00:00:00".to_string(),
            last_modified: "2024-01-06T00:00:00Z".to_string(),
            device_id: "device-1".to_string(),
            device_name: "Reader".to_string(),
        }),
        deleted: false,
        file_hash: "hash".to_string(),
        oneshot: true,
    }
}

#[test]
fn book_dto_matches_kotlin_field_shape_and_formats() {
    let payload =
        serde_json::to_value(BookDto::from_read_model(&book(), true).expect("book should map"))
            .expect("book should serialize");

    assert_eq!(
        payload
            .as_object()
            .expect("book should be an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "created",
            "deleted",
            "fileHash",
            "fileLastModified",
            "id",
            "lastModified",
            "libraryId",
            "media",
            "metadata",
            "name",
            "number",
            "oneshot",
            "readProgress",
            "seriesId",
            "seriesTitle",
            "size",
            "sizeBytes",
            "url",
        ]
    );
    assert_eq!(payload["url"], json!("/library root/books/book 1.cbz"));
    assert_eq!(payload["created"], json!("2024-01-01T00:00:00Z"));
    assert_eq!(payload["fileLastModified"], json!("2024-01-03T00:00:00Z"));
    assert_eq!(payload["size"], json!("1.5 KiB"));
    assert_eq!(
        payload["media"],
        json!({
            "status": "READY",
            "mediaType": "application/epub+zip",
            "pagesCount": 5,
            "comment": "ok",
            "epubDivinaCompatible": true,
            "epubIsKepub": true,
            "mediaProfile": "EPUB"
        })
    );
    assert_eq!(payload["metadata"]["releaseDate"], json!("2024-01-15"));
    assert_eq!(
        payload["readProgress"]["readDate"],
        json!("2024-01-06T00:00:00Z")
    );
}

#[test]
fn book_dto_restricts_non_admin_url_to_filename() {
    let payload =
        serde_json::to_value(BookDto::from_read_model(&book(), false).expect("book should map"))
            .expect("book should serialize");

    assert_eq!(payload["url"], json!("book 1.cbz"));
}

#[test]
fn book_dto_rejects_invalid_wire_dates() {
    let mut invalid = book();
    invalid.created = "not-a-date".to_string();

    assert!(BookDto::from_read_model(&invalid, true).is_err());
}
