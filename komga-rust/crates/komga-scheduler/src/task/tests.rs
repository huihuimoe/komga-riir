use komga_types::{BookMetadataPatchCapability, CopyMode, ImportRequest};

use super::{SUPPORTED_CLASSES, Task};

#[test]
fn reads_kotlin_library_and_book_rows_and_writes_the_same_class() {
    // Jackson writes every Kotlin property; only constructor parameters are read back.
    for (class, kotlin, task, unique_id) in [
        (
            "org.gotson.komga.application.tasks.Task$EmptyTrash",
            r#"{"libraryId":"0S2DXKNMNCWS7","priority":6,"groupId":null,"uniqueId":"EMPTY_TRASH_0S2DXKNMNCWS7"}"#,
            Task::empty_trash("0S2DXKNMNCWS7".into()).high(),
            "EMPTY_TRASH_0S2DXKNMNCWS7",
        ),
        (
            "org.gotson.komga.application.tasks.Task$ScanLibrary",
            r#"{"libraryId":"0S2DXKNMNCWS7","scanDeep":true,"priority":8,"groupId":null,"uniqueId":"SCAN_LIBRARY_0S2DXKNMNCWS7_DEEP_true"}"#,
            Task::scan_library("0S2DXKNMNCWS7".into(), true).highest(),
            "SCAN_LIBRARY_0S2DXKNMNCWS7_DEEP_true",
        ),
        (
            "org.gotson.komga.application.tasks.Task$AnalyzeBook",
            r#"{"bookId":"0S2DXKNMNCWS9","priority":4,"groupId":"0S2DXKNMNCWS8","uniqueId":"ANALYZE_BOOK_0S2DXKNMNCWS9"}"#,
            Task::analyze_book("0S2DXKNMNCWS9".into(), "0S2DXKNMNCWS8".into()),
            "ANALYZE_BOOK_0S2DXKNMNCWS9",
        ),
        (
            "org.gotson.komga.application.tasks.Task$GenerateBookThumbnail",
            r#"{"bookId":"0S2DXKNMNCWS9","priority":5,"groupId":null,"uniqueId":"GENERATE_BOOK_THUMBNAIL_0S2DXKNMNCWS9"}"#,
            Task::generate_book_thumbnail("0S2DXKNMNCWS9".into()).with_priority(5),
            "GENERATE_BOOK_THUMBNAIL_0S2DXKNMNCWS9",
        ),
    ] {
        assert_round_trips(class, kotlin, &task, unique_id);
    }
}

#[test]
fn reads_kotlin_metadata_and_artwork_rows_and_writes_the_same_class() {
    for (class, kotlin, task, unique_id) in [
        (
            "org.gotson.komga.application.tasks.Task$RefreshBookLocalArtwork",
            r#"{"bookId":"0S2DXKNMNCWS9","priority":4,"groupId":null,"uniqueId":"REFRESH_BOOK_LOCAL_ARTWORK_0S2DXKNMNCWS9"}"#,
            Task::refresh_book_local_artwork("0S2DXKNMNCWS9".into()),
            "REFRESH_BOOK_LOCAL_ARTWORK_0S2DXKNMNCWS9",
        ),
        (
            "org.gotson.komga.application.tasks.Task$RefreshBookMetadata",
            r#"{"bookId":"0S2DXKNMNCWS9","capabilities":["TITLE","NUMBER"],"priority":3,"groupId":"0S2DXKNMNCWS8","uniqueId":"REFRESH_BOOK_METADATA_0S2DXKNMNCWS9"}"#,
            Task::refresh_book_metadata(
                "0S2DXKNMNCWS9".into(),
                "0S2DXKNMNCWS8".into(),
                vec![
                    BookMetadataPatchCapability::Title,
                    BookMetadataPatchCapability::Number,
                ],
            )
            .with_priority(3),
            "REFRESH_BOOK_METADATA_0S2DXKNMNCWS9",
        ),
        (
            "org.gotson.komga.application.tasks.Task$RefreshSeriesMetadata",
            r#"{"seriesId":"0S2DXKNMNCWS8","priority":3,"groupId":"0S2DXKNMNCWS8","uniqueId":"REFRESH_SERIES_METADATA_0S2DXKNMNCWS8"}"#,
            Task::refresh_series_metadata("0S2DXKNMNCWS8".into()).with_priority(3),
            "REFRESH_SERIES_METADATA_0S2DXKNMNCWS8",
        ),
        (
            "org.gotson.komga.application.tasks.Task$RefreshSeriesLocalArtwork",
            r#"{"seriesId":"0S2DXKNMNCWS8","priority":4,"groupId":null,"uniqueId":"REFRESH_SERIES_LOCAL_ARTWORK_0S2DXKNMNCWS8"}"#,
            Task::refresh_series_local_artwork("0S2DXKNMNCWS8".into()),
            "REFRESH_SERIES_LOCAL_ARTWORK_0S2DXKNMNCWS8",
        ),
        (
            "org.gotson.komga.application.tasks.Task$FindBookThumbnailsToRegenerate",
            r#"{"forBiggerResultOnly":true,"priority":0,"groupId":null,"uniqueId":"FIND_BOOK_THUMBNAILS_TO_REGENERATE"}"#,
            Task::find_book_thumbnails_to_regenerate(true).lowest(),
            "FIND_BOOK_THUMBNAILS_TO_REGENERATE",
        ),
    ] {
        assert_round_trips(class, kotlin, &task, unique_id);
    }
}

#[test]
fn reads_kotlin_import_rows_and_writes_the_same_class() {
    assert_round_trips(
        "org.gotson.komga.application.tasks.Task$ImportBook",
        r#"{"sourceFile":"/import/Watchmen 01.cbz","seriesId":"0S2DXKNMNCWS8","copyMode":"HARDLINK","destinationName":"Watchmen v01","upgradeBookId":null,"priority":8,"groupId":"0S2DXKNMNCWS8","uniqueId":"IMPORT_BOOK_0S2DXKNMNCWS8_/import/Watchmen 01.cbz"}"#,
        &Task::import_book(ImportRequest {
            source_file: "/import/Watchmen 01.cbz".to_owned(),
            series_id: "0S2DXKNMNCWS8".into(),
            copy_mode: CopyMode::Hardlink,
            destination_name: Some("Watchmen v01".to_owned()),
            upgrade_book_id: None,
        })
        .highest(),
        "IMPORT_BOOK_0S2DXKNMNCWS8_/import/Watchmen 01.cbz",
    );
}

#[test]
fn reads_kotlin_series_aggregation_rows_and_writes_the_same_class() {
    assert_round_trips(
        "org.gotson.komga.application.tasks.Task$AggregateSeriesMetadata",
        r#"{"seriesId":"0S2DXKNMNCWS8","priority":4,"groupId":"0S2DXKNMNCWS8","uniqueId":"AGGREGATE_SERIES_METADATA_0S2DXKNMNCWS8"}"#,
        &Task::aggregate_series_metadata("0S2DXKNMNCWS8".into()),
        "AGGREGATE_SERIES_METADATA_0S2DXKNMNCWS8",
    );
}

#[test]
fn reads_kotlin_series_deletion_rows_without_a_group() {
    let task = Task::delete_series("0S2DXKNMNCWS8".into()).highest();
    assert_round_trips(
        "org.gotson.komga.application.tasks.Task$DeleteSeries",
        r#"{"seriesId":"0S2DXKNMNCWS8","priority":8,"groupId":null,"uniqueId":"DELETE_SERIES_0S2DXKNMNCWS8"}"#,
        &task,
        "DELETE_SERIES_0S2DXKNMNCWS8",
    );
    assert_eq!(task.to_row().group_id, None);
}

#[test]
fn reads_kotlin_hashing_rows_and_writes_the_same_class() {
    for (class, kotlin, task, unique_id) in [
        (
            "org.gotson.komga.application.tasks.Task$HashBook",
            r#"{"bookId":"0S2DXKNMNCWS9","priority":0,"groupId":null,"uniqueId":"HASH_BOOK_0S2DXKNMNCWS9"}"#,
            Task::hash_book("0S2DXKNMNCWS9".into()).lowest(),
            "HASH_BOOK_0S2DXKNMNCWS9",
        ),
        (
            "org.gotson.komga.application.tasks.Task$HashBookKoreader",
            r#"{"bookId":"0S2DXKNMNCWS9","priority":0,"groupId":null,"uniqueId":"HASH_BOOK_KOREADER_0S2DXKNMNCWS9"}"#,
            Task::hash_book_koreader("0S2DXKNMNCWS9".into()).lowest(),
            "HASH_BOOK_KOREADER_0S2DXKNMNCWS9",
        ),
        (
            "org.gotson.komga.application.tasks.Task$FindBooksWithMissingPageHash",
            r#"{"libraryId":"0S2DXKNMNCWS7","priority":0,"groupId":null,"uniqueId":"FIND_BOOKS_WITH_MISSING_PAGE_HASH_0S2DXKNMNCWS7"}"#,
            Task::find_books_with_missing_page_hash("0S2DXKNMNCWS7".into()).lowest(),
            "FIND_BOOKS_WITH_MISSING_PAGE_HASH_0S2DXKNMNCWS7",
        ),
        (
            "org.gotson.komga.application.tasks.Task$HashBookPages",
            r#"{"bookId":"0S2DXKNMNCWS9","priority":1,"groupId":null,"uniqueId":"HASH_BOOK_PAGES_0S2DXKNMNCWS9"}"#,
            Task::hash_book_pages("0S2DXKNMNCWS9".into()).with_priority(1),
            "HASH_BOOK_PAGES_0S2DXKNMNCWS9",
        ),
    ] {
        assert_round_trips(class, kotlin, &task, unique_id);
    }
}

#[test]
fn reads_kotlin_index_maintenance_rows_and_writes_the_same_class() {
    for (class, kotlin, task, unique_id) in [
        (
            "org.gotson.komga.application.tasks.Task$RebuildIndex",
            r#"{"entities":["Series"],"priority":8,"groupId":null,"uniqueId":"REBUILD_INDEX"}"#,
            Task::rebuild_index().highest(),
            "REBUILD_INDEX",
        ),
        (
            "org.gotson.komga.application.tasks.Task$UpgradeIndex",
            r#"{"priority":8,"groupId":null,"uniqueId":"UPGRADE_INDEX"}"#,
            Task::upgrade_index().highest(),
            "UPGRADE_INDEX",
        ),
    ] {
        assert_round_trips(class, kotlin, &task, unique_id);
    }
    assert_eq!(
        Task::rebuild_index().highest().to_row().payload,
        r#"{"entities":null,"priority":8}"#
    );
}

/// `kotlin`, the Jackson payload of a `class` row, decodes to `task`, which writes the same class
/// under `unique_id` and decodes back from its own row.
fn assert_round_trips(class: &str, kotlin: &str, task: &Task, unique_id: &str) {
    assert_eq!(&Task::decode(class, kotlin).expect("Kotlin payload"), task);
    let row = task.to_row();
    assert_eq!((row.id.as_str(), row.class.as_str()), (unique_id, class));
    assert!(SUPPORTED_CLASSES.contains(&class));
    assert_eq!(
        &Task::decode(&row.class, &row.payload).expect("Own payload"),
        task
    );
}

#[test]
fn scan_follow_ups_carry_the_kotlin_constructor_parameters() {
    let cases = [
        (
            Task::find_books_to_convert("L".into()).lowest(),
            "FIND_BOOKS_TO_CONVERT_L",
            None,
            r#"{"libraryId":"L","priority":0}"#,
        ),
        (
            Task::convert_book("B".into(), "S".into()).with_priority(1),
            "CONVERT_BOOK_B",
            Some("S"),
            r#"{"bookId":"B","priority":1,"groupId":"S"}"#,
        ),
        (
            Task::find_duplicate_pages_to_delete("L".into()).lowest(),
            "FIND_DUPLICATE_PAGES_TO_DELETE_L",
            None,
            r#"{"libraryId":"L","priority":0}"#,
        ),
        (
            Task::repair_extension("B".into(), "S".into()).low(),
            "REPAIR_EXTENSION_B",
            Some("S"),
            r#"{"bookId":"B","priority":2,"groupId":"S"}"#,
        ),
    ];
    for (task, id, group, payload) in cases {
        let row = task.to_row();
        assert_eq!(
            (row.id.as_str(), row.group_id.as_deref()),
            (id, group),
            "{task:?}"
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.payload).expect("Valid JSON payload"),
            serde_json::from_str::<serde_json::Value>(payload).expect("Expected JSON"),
            "{task:?}"
        );
        assert_eq!(
            row.class,
            format!(
                "org.gotson.komga.application.tasks.Task${}",
                row.simple_type
            )
        );
        assert!(SUPPORTED_CLASSES.contains(&row.class.as_str()), "{task:?}");
        assert_eq!(
            Task::decode(&row.class, &row.payload).expect("Own payload"),
            task
        );
    }
}

#[test]
fn rejects_unknown_classes_and_malformed_payloads() {
    assert!(Task::decode("org.gotson.komga.application.tasks.Task$AnalyzeBook", "{}").is_err());
    assert!(
        Task::decode(
            "org.gotson.komga.application.tasks.Task$EmptyTrash",
            r#"{"priority":6}"#
        )
        .is_err()
    );
}
