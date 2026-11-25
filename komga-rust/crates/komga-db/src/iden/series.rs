//! Series, series metadata, book metadata aggregations, series thumbnails and sidecars.

use sea_query::Iden;

/// `SERIES` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SERIES"]
pub enum SeriesTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "LIBRARY_ID"]
    LibraryId,
    #[iden = "NAME"]
    Name,
    #[iden = "URL"]
    Url,
    #[iden = "FILE_LAST_MODIFIED"]
    FileLastModified,
    #[iden = "BOOK_COUNT"]
    BookCount,
    #[iden = "DELETED_DATE"]
    DeletedDate,
    #[iden = "ONESHOT"]
    Oneshot,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `BOOK_METADATA_AGGREGATION` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK_METADATA_AGGREGATION"]
pub enum BookMetadataAggregationTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "RELEASE_DATE"]
    ReleaseDate,
    #[iden = "SUMMARY"]
    Summary,
    #[iden = "SUMMARY_NUMBER"]
    SummaryNumber,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `BOOK_METADATA_AGGREGATION_AUTHOR` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK_METADATA_AGGREGATION_AUTHOR"]
pub enum BookMetadataAggregationAuthorTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "NAME"]
    Name,
    #[iden = "ROLE"]
    Role,
}

/// `BOOK_METADATA_AGGREGATION_NOTABLE_ENTITIES` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK_METADATA_AGGREGATION_NOTABLE_ENTITIES"]
pub enum BookMetadataAggregationNotableEntitiesTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "KIND"]
    Kind,
    #[iden = "NAME"]
    Name,
}

/// `BOOK_METADATA_AGGREGATION_TAG` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK_METADATA_AGGREGATION_TAG"]
pub enum BookMetadataAggregationTagTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "TAG"]
    Tag,
}

/// `SIDECAR` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SIDECAR"]
pub enum SidecarTable {
    Table,
    #[iden = "URL"]
    Url,
    #[iden = "PARENT_URL"]
    ParentUrl,
    #[iden = "LAST_MODIFIED_TIME"]
    LastModifiedTime,
    #[iden = "LIBRARY_ID"]
    LibraryId,
}

/// `THUMBNAIL_SERIES` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "THUMBNAIL_SERIES"]
pub enum ThumbnailSeriesTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "THUMBNAIL"]
    Thumbnail,
    #[iden = "URL"]
    Url,
    #[iden = "SELECTED"]
    Selected,
    #[iden = "TYPE"]
    Type,
    #[iden = "MEDIA_TYPE"]
    MediaType,
    #[iden = "WIDTH"]
    Width,
    #[iden = "HEIGHT"]
    Height,
    #[iden = "FILE_SIZE"]
    FileSize,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `SERIES_METADATA` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SERIES_METADATA"]
pub enum SeriesMetadataTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "AGE_RATING"]
    AgeRating,
    #[iden = "STATUS"]
    Status,
    #[iden = "TITLE"]
    Title,
    #[iden = "TITLE_SORT"]
    TitleSort,
    #[iden = "TITLE_LOCK"]
    TitleLock,
    #[iden = "TITLE_SORT_LOCK"]
    TitleSortLock,
    #[iden = "LANGUAGE"]
    Language,
    #[iden = "READING_DIRECTION"]
    ReadingDirection,
    #[iden = "STATUS_LOCK"]
    StatusLock,
    #[iden = "SUMMARY"]
    Summary,
    #[iden = "SUMMARY_LOCK"]
    SummaryLock,
    #[iden = "READING_DIRECTION_LOCK"]
    ReadingDirectionLock,
    #[iden = "PUBLISHER"]
    Publisher,
    #[iden = "PUBLISHER_LOCK"]
    PublisherLock,
    #[iden = "AGE_RATING_LOCK"]
    AgeRatingLock,
    #[iden = "LANGUAGE_LOCK"]
    LanguageLock,
    #[iden = "GENRES_LOCK"]
    GenresLock,
    #[iden = "TOTAL_BOOK_COUNT"]
    TotalBookCount,
    #[iden = "TOTAL_BOOK_COUNT_LOCK"]
    TotalBookCountLock,
    #[iden = "TAGS_LOCK"]
    TagsLock,
    #[iden = "SHARING_LABELS_LOCK"]
    SharingLabelsLock,
    #[iden = "LINKS_LOCK"]
    LinksLock,
    #[iden = "ALTERNATE_TITLES_LOCK"]
    AlternateTitlesLock,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `SERIES_METADATA_GENRE` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SERIES_METADATA_GENRE"]
pub enum SeriesMetadataGenreTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "GENRE"]
    Genre,
}

/// `SERIES_METADATA_TAG` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SERIES_METADATA_TAG"]
pub enum SeriesMetadataTagTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "TAG"]
    Tag,
}

/// `SERIES_AND_BOOK_TAG` view column identifiers: series tags and aggregated book tags.
#[derive(Iden, Clone, Copy)]
#[iden = "SERIES_AND_BOOK_TAG"]
pub enum SeriesAndBookTagView {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "TAG"]
    Tag,
}

/// `SERIES_METADATA_LINK` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SERIES_METADATA_LINK"]
pub enum SeriesMetadataLinkTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "LABEL"]
    Label,
    #[iden = "URL"]
    Url,
}

/// `SERIES_METADATA_ALTERNATE_TITLE` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SERIES_METADATA_ALTERNATE_TITLE"]
pub enum SeriesMetadataAlternateTitleTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "LABEL"]
    Label,
    #[iden = "TITLE"]
    Title,
}

/// `SERIES_METADATA_SHARING` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SERIES_METADATA_SHARING"]
pub enum SeriesMetadataSharingTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "LABEL"]
    Label,
}
