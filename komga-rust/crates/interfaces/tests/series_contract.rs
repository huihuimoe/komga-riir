use komga_application::discovery::{
    SeriesAlternateTitleRecord, SeriesMetadataLinkRecord, SeriesReadModel, SeriesReadingDirection,
};
use komga_domain::discovery::SeriesStatus;
use komga_interfaces::contracts::discovery::{SeriesAlphabeticalGroupDto, SeriesDto};
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
        status_lock: true,
        title_lock: true,
        title_sort_lock: true,
        summary_lock: true,
        reading_direction_lock: true,
        publisher_lock: true,
        age_rating_lock: true,
        language_lock: true,
        genres_lock: true,
        tags_lock: true,
        total_book_count: Some(5),
        total_book_count_lock: true,
        sharing_labels_lock: true,
        links: vec![SeriesMetadataLinkRecord {
            label: "Wiki".to_string(),
            url: "https://example.com".to_string(),
        }],
        links_lock: true,
        alternate_titles: vec![SeriesAlternateTitleRecord {
            label: "en".to_string(),
            title: "Alt Title".to_string(),
        }],
        alternate_titles_lock: true,
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
    for (field, expected) in [
        ("statusLock", json!(true)),
        ("titleLock", json!(true)),
        ("titleSortLock", json!(true)),
        ("summaryLock", json!(true)),
        ("readingDirectionLock", json!(true)),
        ("publisherLock", json!(true)),
        ("ageRatingLock", json!(true)),
        ("languageLock", json!(true)),
        ("genresLock", json!(true)),
        ("tagsLock", json!(true)),
        ("totalBookCount", json!(5)),
        ("totalBookCountLock", json!(true)),
        ("sharingLabelsLock", json!(true)),
        ("linksLock", json!(true)),
        ("alternateTitlesLock", json!(true)),
    ] {
        assert_eq!(payload["metadata"][field], expected);
    }
    assert_eq!(
        payload["metadata"]["links"],
        json!([{ "label": "Wiki", "url": "https://example.com" }])
    );
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

#[test]
fn series_alphabetical_group_dto_uses_explicit_wire_fields() {
    let payload = serde_json::to_value(SeriesAlphabeticalGroupDto {
        group: "A".to_string(),
        count: 3,
    })
    .expect("alphabetical group should serialize");

    assert_eq!(payload, json!({"group": "A", "count": 3}));
}
