//! Verifies that `crate::iden` stays 1:1 with the real migrated SQLite schemas
//! (`main`, `era` and `tasks` migrations).

use std::collections::{BTreeMap, BTreeSet};

use crate::iden::*;
use komga_test_support::tempdir;
use sea_query::Iden;
use sqlx::{Pool, Sqlite};

use crate::Database;

/// Migration bookkeeping tables intentionally excluded from `crate::iden`.
const IGNORED_META_TABLES: &[&str] = &["flyway_schema_history", "ERA_SCHEMA_HISTORY"];

macro_rules! iden_entry {
    ($ty:ty => [ $($col:ident),* $(,)? ]) => {{
        // Exhaustive match fails to compile if a variant is added to `$ty` without registering it.
        #[allow(dead_code)]
        const fn assert_exhaustive(variant: $ty) {
            match variant {
                <$ty>::Table => {}
                $( <$ty>::$col => {} )*
            }
        }

        let table = Iden::to_string(&<$ty>::Table);
        let cols: BTreeSet<String> = [ $( Iden::to_string(&<$ty>::$col) ),* ]
            .into_iter()
            .collect();
        (table, cols)
    }};
}

async fn introspect_schema(pool: &Pool<Sqlite>) -> BTreeMap<String, BTreeSet<String>> {
    let objects: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master \
         WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(pool)
    .await
    .expect("query sqlite_master");

    let mut schema = BTreeMap::new();
    for name in objects {
        if IGNORED_META_TABLES.contains(&name.as_str()) {
            continue;
        }
        let raw_cols: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_xinfo(?)")
            .bind(&name)
            .fetch_all(pool)
            .await
            .expect("query pragma_table_xinfo");
        // SQLite identifiers are case-insensitive; migrations mix case (e.g. `BOOK.oneshot`)
        // while `crate::iden` normalizes to uppercase.
        let cols = raw_cols
            .into_iter()
            .map(|col| col.to_ascii_uppercase())
            .collect();
        schema.insert(name.to_ascii_uppercase(), cols);
    }
    schema
}

type TableSchema = (String, BTreeSet<String>);

fn book_and_media_tables() -> Vec<TableSchema> {
    vec![
        iden_entry!(BookTable => [
            Id, Name, Url, Number, FileLastModified, FileSize, FileHash, FileHashKoreader,
            LibraryId, SeriesId, DeletedDate, Oneshot, CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(BookProjectionTable => [
            BookId, Profile, FileSize, CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(MediaTable => [
            BookId, Status, MediaType, Comment, PageCount, EpubDivinaCompatible, EpubIsKepub,
            ExtensionClass, ExtensionValueBlob, Unused, CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(MediaPageTable => [
            BookId, FileName, MediaType, Number, Width, Height, FileHash, FileSize,
        ]),
        iden_entry!(MediaFileTable => [
            BookId, FileName, MediaType, SubType, FileSize,
        ]),
        iden_entry!(BookMetadataTable => [
            BookId, Title, TitleLock, Summary, SummaryLock, Number, NumberLock, NumberSort,
            NumberSortLock, ReleaseDate, ReleaseDateLock, AuthorsLock, NotableEntitiesLock,
            TagsLock, Isbn, IsbnLock, LinksLock, CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(BookMetadataAuthorTable => [BookId, Name, Role]),
        iden_entry!(BookMetadataTagTable => [BookId, Tag]),
        iden_entry!(BookMetadataNotableEntitiesTable => [BookId, Kind, Name]),
        iden_entry!(BookMetadataLinkTable => [BookId, Label, Url]),
        iden_entry!(BookMetadataAggregationTable => [
            SeriesId, ReleaseDate, Summary, SummaryNumber, CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(BookMetadataAggregationAuthorTable => [SeriesId, Name, Role]),
        iden_entry!(BookMetadataAggregationNotableEntitiesTable => [SeriesId, Kind, Name]),
        iden_entry!(BookMetadataAggregationTagTable => [SeriesId, Tag]),
        iden_entry!(ThumbnailBookTable => [
            Id, BookId, Thumbnail, Url, Selected, Type, MediaType, Width, Height, FileSize,
            CreatedDate, LastModifiedDate,
        ]),
    ]
}

fn series_and_event_tables() -> Vec<TableSchema> {
    vec![
        iden_entry!(SeriesTable => [
            Id, LibraryId, Name, Url, FileLastModified, BookCount, DeletedDate, Oneshot,
            CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(SeriesMetadataTable => [
            SeriesId, AgeRating, Status, Title, TitleSort, TitleLock, TitleSortLock, Language,
            ReadingDirection, StatusLock, Summary, SummaryLock, ReadingDirectionLock, Publisher,
            PublisherLock, AgeRatingLock, LanguageLock, GenresLock, TotalBookCount,
            TotalBookCountLock, TagsLock, SharingLabelsLock, LinksLock, AlternateTitlesLock,
            CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(SeriesMetadataGenreTable => [SeriesId, Genre]),
        iden_entry!(SeriesMetadataTagTable => [SeriesId, Tag]),
        iden_entry!(SeriesAndBookTagView => [SeriesId, Tag]),
        iden_entry!(SeriesMetadataLinkTable => [SeriesId, Label, Url]),
        iden_entry!(SeriesMetadataAlternateTitleTable => [SeriesId, Label, Title]),
        iden_entry!(SeriesMetadataSharingTable => [SeriesId, Label]),
        iden_entry!(ThumbnailSeriesTable => [
            Id, SeriesId, Thumbnail, Url, Selected, Type, MediaType, Width, Height, FileSize,
            CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(SidecarTable => [Url, ParentUrl, LastModifiedTime, LibraryId]),
        iden_entry!(PageHashTable => [
            Hash, Size, Action, DeleteCount, CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(PageHashThumbnailTable => [Hash, Thumbnail]),
        iden_entry!(HistoricalEventTable => [Id, Type, BookId, SeriesId, Timestamp]),
        iden_entry!(HistoricalEventPropertiesTable => [Id, Key, Value]),
    ]
}

fn set_and_library_tables() -> Vec<TableSchema> {
    vec![
        // set.rs
        iden_entry!(CollectionTable => [
            Id, Name, Ordered, SeriesCount, CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(CollectionSeriesTable => [CollectionId, SeriesId, Number]),
        iden_entry!(ThumbnailCollectionTable => [
            Id, CollectionId, Thumbnail, Selected, Type, MediaType, Width, Height, FileSize,
            CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(ReadlistTable => [
            Id, Name, Summary, Ordered, BookCount, CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(ReadlistBookTable => [ReadlistId, BookId, Number]),
        iden_entry!(ThumbnailReadlistTable => [
            Id, ReadlistId, Thumbnail, Selected, Type, MediaType, Width, Height, FileSize,
            CreatedDate, LastModifiedDate,
        ]),
        // library.rs
        iden_entry!(LibraryTable => [
            Id, Name, Root, ImportComicInfoBook, ImportComicInfoSeries,
            ImportComicInfoCollection, ImportComicInfoReadList,
            ImportComicInfoSeriesAppendVolume, ImportEpubBook, ImportEpubSeries,
            ImportMylarSeries, ImportLocalArtwork, ImportBarcodeIsbn, ScanForceModifiedTime,
            ScanOnStartup, ScanInterval, ScanCbx, ScanPdf, ScanEpub, RepairExtensions,
            ConvertToCbz, EmptyTrashAfterScan, SeriesCover, HashFiles, HashPages, HashKoreader,
            AnalyzeDimensions, OneshotsDirectory, UnavailableDate, CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(LibraryExclusionsTable => [LibraryId, Exclusion]),
        // server.rs
        iden_entry!(ServerSettingsTable => [Key, Value]),
    ]
}

fn user_and_runtime_tables() -> Vec<TableSchema> {
    vec![
        // user.rs
        iden_entry!(UserTable => [
            Id, CreatedDate, LastModifiedDate, Email, Password, SharedAllLibraries,
            AgeRestriction, AgeRestrictionAllowOnly,
        ]),
        iden_entry!(UserRoleTable => [UserId, Role]),
        iden_entry!(UserApiKeyTable => [
            Id, UserId, ApiKey, Comment, CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(AuthenticationActivityTable => [
            UserId, Email, Ip, UserAgent, Success, Error, DateTime, Source, ApiKeyId,
            ApiKeyComment,
        ]),
        iden_entry!(ClientSettingsGlobalTable => [Key, Value, AllowUnauthorized]),
        iden_entry!(ClientSettingsUserTable => [UserId, Key, Value]),
        iden_entry!(ReadProgressTable => [
            BookId, UserId, Page, Completed, ReadDate, DeviceId, DeviceName, Locator,
            CreatedDate, LastModifiedDate,
        ]),
        iden_entry!(ReadProgressSeriesTable => [
            SeriesId, UserId, ReadCount, InProgressCount, MostRecentReadDate, LastModifiedDate,
        ]),
        iden_entry!(UserLibrarySharingTable => [UserId, LibraryId]),
        iden_entry!(UserSharingTable => [UserId, Label, Allow]),
        iden_entry!(AnnouncementsReadTable => [UserId, AnnouncementId]),
        iden_entry!(SyncPointTable => [Id, UserId, ApiKeyId, CreatedDate]),
        iden_entry!(SyncPointBookTable => [
            SyncPointId, BookId, BookCreatedDate, BookLastModifiedDate, BookFileLastModified,
            BookFileSize, BookFileHash, BookMetadataLastModifiedDate,
            BookReadProgressLastModifiedDate, BookThumbnailId, Synced,
        ]),
        iden_entry!(SyncPointBookRemovedSyncedTable => [SyncPointId, BookId]),
        iden_entry!(SyncPointReadListTable => [
            SyncPointId, ReadListId, ReadListName, ReadListCreatedDate,
            ReadListLastModifiedDate, Synced,
        ]),
        iden_entry!(SyncPointReadListBookTable => [SyncPointId, ReadListId, BookId]),
        iden_entry!(SyncPointReadListRemovedSyncedTable => [SyncPointId, ReadListId]),
        // search.rs, file_operation.rs & task.rs (`era` migrations)
        iden_entry!(SearchIndexPendingTable => [Seq, Entity, EntityId]),
        iden_entry!(FileOperationTable => [
            Id, SourceUrl, SourceSize, SourceModified, TargetUrl, ReplacedUrl, OutputSize,
            RemoveSource, OutputModified, Committed,
        ]),
        iden_entry!(TaskHandoverTable => [
            Seq, Id, Priority, GroupId, Class, SimpleType, Payload,
        ]),
        // task.rs (tasks migrations)
        iden_entry!(TaskTable => [
            Id, Priority, GroupId, Class, SimpleType, Payload, Owner, CreatedDate,
            LastModifiedDate,
        ]),
    ]
}

fn modeled_schema() -> BTreeMap<String, BTreeSet<String>> {
    let groups = [
        book_and_media_tables(),
        series_and_event_tables(),
        set_and_library_tables(),
        user_and_runtime_tables(),
    ];
    let mut map = BTreeMap::new();
    for (table, cols) in groups.into_iter().flatten() {
        assert!(
            map.insert(table.clone(), cols).is_none(),
            "duplicate iden entry for table {table}"
        );
    }
    map
}

#[tokio::test]
async fn iden_enums_match_migrated_sqlite_schema() {
    let dir = tempdir().expect("create tempdir");
    let main_db = Database::connect(&dir.path().join("database.sqlite"))
        .await
        .expect("connect main db");
    let tasks_db = Database::connect(&dir.path().join("tasks.sqlite"))
        .await
        .expect("connect tasks db");
    Database::migrate_all(&main_db, &tasks_db, &dir.path().join("backups"))
        .await
        .expect("migrate main and tasks databases");

    let mut actual = introspect_schema(main_db.pool()).await;
    for (table, cols) in introspect_schema(tasks_db.pool()).await {
        assert!(
            actual.insert(table.clone(), cols).is_none(),
            "table {table} exists in both main and tasks databases"
        );
    }

    let expected = modeled_schema();

    assert_eq!(
        expected.keys().collect::<Vec<_>>(),
        actual.keys().collect::<Vec<_>>(),
        "mismatch between crate::iden tables and migrated SQLite schema"
    );

    for (table, actual_cols) in &actual {
        assert_eq!(
            &expected[table], actual_cols,
            "column mismatch for table {table}"
        );
    }
}
