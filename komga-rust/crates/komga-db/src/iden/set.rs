//! Collections, read lists, their memberships, and thumbnails.

use sea_query::Iden;

/// `COLLECTION` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "COLLECTION"]
pub enum CollectionTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "NAME"]
    Name,
    #[iden = "ORDERED"]
    Ordered,
    #[iden = "SERIES_COUNT"]
    SeriesCount,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `COLLECTION_SERIES` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "COLLECTION_SERIES"]
pub enum CollectionSeriesTable {
    Table,
    #[iden = "COLLECTION_ID"]
    CollectionId,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "NUMBER"]
    Number,
}

/// `THUMBNAIL_COLLECTION` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "THUMBNAIL_COLLECTION"]
pub enum ThumbnailCollectionTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "COLLECTION_ID"]
    CollectionId,
    #[iden = "THUMBNAIL"]
    Thumbnail,
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

/// `READLIST` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "READLIST"]
pub enum ReadlistTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "NAME"]
    Name,
    #[iden = "SUMMARY"]
    Summary,
    #[iden = "ORDERED"]
    Ordered,
    #[iden = "BOOK_COUNT"]
    BookCount,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `READLIST_BOOK` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "READLIST_BOOK"]
pub enum ReadlistBookTable {
    Table,
    #[iden = "READLIST_ID"]
    ReadlistId,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "NUMBER"]
    Number,
}

/// `THUMBNAIL_READLIST` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "THUMBNAIL_READLIST"]
pub enum ThumbnailReadlistTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "READLIST_ID"]
    ReadlistId,
    #[iden = "THUMBNAIL"]
    Thumbnail,
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

/// Collection and read-list tables with their member and thumbnail tables.
#[derive(Iden, Clone, Copy)]
pub enum SetTable {
    #[iden = "COLLECTION"]
    Collection,
    #[iden = "COLLECTION_SERIES"]
    CollectionSeries,
    #[iden = "THUMBNAIL_COLLECTION"]
    ThumbnailCollection,
    #[iden = "READLIST"]
    Readlist,
    #[iden = "READLIST_BOOK"]
    ReadlistBook,
    #[iden = "THUMBNAIL_READLIST"]
    ThumbnailReadlist,
}

#[derive(Iden, Clone, Copy)]
pub enum SetKey {
    #[iden = "ID"]
    Id,
    #[iden = "COLLECTION_ID"]
    CollectionId,
    #[iden = "READLIST_ID"]
    ReadlistId,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
    #[iden = "NAME"]
    Name,
    #[iden = "SUMMARY"]
    Summary,
    #[iden = "ORDERED"]
    Ordered,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "BOOK_COUNT"]
    BookCount,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "NUMBER"]
    Number,
    #[iden = "SERIES_COUNT"]
    SeriesCount,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "THUMBNAIL"]
    Thumbnail,
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
}
