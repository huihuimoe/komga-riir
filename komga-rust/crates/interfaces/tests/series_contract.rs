use komga_application::discovery::{SeriesReadModel, SeriesReadingDirection};
use komga_domain::discovery::SeriesStatus;
use komga_interfaces::contracts::discovery::SeriesDto;
use serde_json::json;

fn series() -> SeriesReadModel {
    SeriesReadModel {
        id: "series-1".to_string(),
        library_id: "library-1".to_string(),
        name: "Series File Name".to_string(),
        url: "file:///data/series/series-1".to_string(),
        title: "Series Title".to_string(),
        title_sort: "Series Sort".to_string(),
        labels: vec!["Team".to_string()],
        created: "2024-01-01 00:00:00".to_string(),
        last_modified: "2024-01-02T00:00:00Z".to_string(),
        file_last_modified: "1704240000".to_string(),
        books_count: 2,
        books_read_count: 1,
        books_unread_count: 1,
        books_in_progress_count: 0,
        status: SeriesStatus::Ongoing,
        summary: "Summary".to_string(),
        reading_direction: Some(SeriesReadingDirection::LeftToRight),
        publisher: "Publisher".to_string(),
        age_rating: Some(13),
        language: "en".to_string(),
        genres: vec!["Drama".to_string()],
        tags: vec!["Favorite".to_string()],
        alternate_titles: vec!["en::Alt Title".to_string()],
        metadata_created: "2024-01-03 00:00:00".to_string(),
        metadata_last_modified: "2024-01-04T00:00:00Z".to_string(),
        books_metadata_authors: vec!["Author::Writer".to_string()],
        books_metadata_tags: vec!["tag".to_string()],
        books_metadata_release_date: Some("2024-01-15".to_string()),
        books_metadata_summary: "Books summary".to_string(),
        books_metadata_summary_number: "2".to_string(),
        books_metadata_created: "2024-01-05 00:00:00".to_string(),
        books_metadata_last_modified: "2024-01-06T00:00:00Z".to_string(),
        deleted: false,
        oneshot: true,
    }
}

#[test]
fn series_dto_matches_kotlin_field_shape_and_formats() {
    let payload = serde_json::to_value(
        SeriesDto::from_read_model(&series(), true).expect("series should map"),
    )
    .expect("series should serialize");

    assert_eq!(
        payload
            .as_object()
            .expect("series should be an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "booksCount",
            "booksInProgressCount",
            "booksMetadata",
            "booksReadCount",
            "booksUnreadCount",
            "created",
            "deleted",
            "fileLastModified",
            "id",
            "lastModified",
            "libraryId",
            "metadata",
            "name",
            "oneshot",
            "url",
        ]
    );
    assert_eq!(payload["url"], json!("/data/series/series-1"));
    assert_eq!(payload["created"], json!("2024-01-01T00:00:00Z"));
    assert_eq!(payload["fileLastModified"], json!("2024-01-03T00:00:00Z"));
    assert_eq!(payload["metadata"]["status"], json!("ONGOING"));
    assert_eq!(
        payload["metadata"]["readingDirection"],
        json!("LEFT_TO_RIGHT")
    );
    assert_eq!(
        payload["metadata"]["alternateTitles"],
        json!([{ "label": "en", "title": "Alt Title" }])
    );
    assert_eq!(
        payload["booksMetadata"]["authors"],
        json!([{ "name": "Author", "role": "Writer" }])
    );
    assert_eq!(payload["booksMetadata"]["releaseDate"], json!("2024-01-15"));

    let restricted = serde_json::to_value(
        SeriesDto::from_read_model(&series(), false).expect("series should map"),
    )
    .expect("series should serialize");
    assert_eq!(restricted["url"], json!(""));
}

#[test]
fn series_dto_rejects_invalid_wire_dates() {
    let mut invalid = series();
    invalid.metadata_created = "not-a-date".to_string();

    assert!(SeriesDto::from_read_model(&invalid, true).is_err());
}
