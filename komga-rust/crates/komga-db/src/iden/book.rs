//! Books, media, book metadata, book thumbnails and page hashes.

use sea_query::Iden;

/// `BOOK` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK"]
pub enum BookTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "NAME"]
    Name,
    #[iden = "URL"]
    Url,
    #[iden = "NUMBER"]
    Number,
    #[iden = "FILE_LAST_MODIFIED"]
    FileLastModified,
    #[iden = "FILE_SIZE"]
    FileSize,
    #[iden = "FILE_HASH"]
    FileHash,
    #[iden = "FILE_HASH_KOREADER"]
    FileHashKoreader,
    #[iden = "LIBRARY_ID"]
    LibraryId,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "DELETED_DATE"]
    DeletedDate,
    #[iden = "ONESHOT"]
    Oneshot,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `BOOK_PROJECTION` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK_PROJECTION"]
pub enum BookProjectionTable {
    Table,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "PROFILE"]
    Profile,
    #[iden = "FILE_SIZE"]
    FileSize,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `MEDIA` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "MEDIA"]
pub enum MediaTable {
    Table,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "STATUS"]
    Status,
    #[iden = "MEDIA_TYPE"]
    MediaType,
    #[iden = "COMMENT"]
    Comment,
    #[iden = "PAGE_COUNT"]
    PageCount,
    #[iden = "EPUB_DIVINA_COMPATIBLE"]
    EpubDivinaCompatible,
    #[iden = "EPUB_IS_KEPUB"]
    EpubIsKepub,
    #[iden = "EXTENSION_CLASS"]
    ExtensionClass,
    #[iden = "EXTENSION_VALUE_BLOB"]
    ExtensionValueBlob,
    /// Legacy `EXTENSION_VALUE` column renamed by `V20231201115954__media_extension_blob.sql`.
    #[iden = "_UNUSED"]
    Unused,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `MEDIA_PAGE` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "MEDIA_PAGE"]
pub enum MediaPageTable {
    Table,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "FILE_NAME"]
    FileName,
    #[iden = "MEDIA_TYPE"]
    MediaType,
    #[iden = "NUMBER"]
    Number,
    #[iden = "WIDTH"]
    Width,
    #[iden = "HEIGHT"]
    Height,
    #[iden = "FILE_HASH"]
    FileHash,
    #[iden = "FILE_SIZE"]
    FileSize,
}

/// `MEDIA_FILE` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "MEDIA_FILE"]
pub enum MediaFileTable {
    Table,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "FILE_NAME"]
    FileName,
    #[iden = "MEDIA_TYPE"]
    MediaType,
    #[iden = "SUB_TYPE"]
    SubType,
    #[iden = "FILE_SIZE"]
    FileSize,
}

/// `BOOK_METADATA` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK_METADATA"]
pub enum BookMetadataTable {
    Table,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "TITLE"]
    Title,
    #[iden = "TITLE_LOCK"]
    TitleLock,
    #[iden = "SUMMARY"]
    Summary,
    #[iden = "SUMMARY_LOCK"]
    SummaryLock,
    #[iden = "NUMBER"]
    Number,
    #[iden = "NUMBER_LOCK"]
    NumberLock,
    #[iden = "NUMBER_SORT"]
    NumberSort,
    #[iden = "NUMBER_SORT_LOCK"]
    NumberSortLock,
    #[iden = "RELEASE_DATE"]
    ReleaseDate,
    #[iden = "RELEASE_DATE_LOCK"]
    ReleaseDateLock,
    #[iden = "AUTHORS_LOCK"]
    AuthorsLock,
    #[iden = "NOTABLE_ENTITIES_LOCK"]
    NotableEntitiesLock,
    #[iden = "TAGS_LOCK"]
    TagsLock,
    #[iden = "ISBN"]
    Isbn,
    #[iden = "ISBN_LOCK"]
    IsbnLock,
    #[iden = "LINKS_LOCK"]
    LinksLock,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `BOOK_METADATA_AUTHOR` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK_METADATA_AUTHOR"]
pub enum BookMetadataAuthorTable {
    Table,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "NAME"]
    Name,
    #[iden = "ROLE"]
    Role,
}

/// `BOOK_METADATA_TAG` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK_METADATA_TAG"]
pub enum BookMetadataTagTable {
    Table,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "TAG"]
    Tag,
}

/// `BOOK_METADATA_NOTABLE_ENTITIES` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK_METADATA_NOTABLE_ENTITIES"]
pub enum BookMetadataNotableEntitiesTable {
    Table,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "KIND"]
    Kind,
    #[iden = "NAME"]
    Name,
}

/// `BOOK_METADATA_LINK` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "BOOK_METADATA_LINK"]
pub enum BookMetadataLinkTable {
    Table,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "LABEL"]
    Label,
    #[iden = "URL"]
    Url,
}

/// `THUMBNAIL_BOOK` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "THUMBNAIL_BOOK"]
pub enum ThumbnailBookTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "BOOK_ID"]
    BookId,
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

/// `PAGE_HASH` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "PAGE_HASH"]
pub enum PageHashTable {
    Table,
    #[iden = "HASH"]
    Hash,
    #[iden = "SIZE"]
    Size,
    #[iden = "ACTION"]
    Action,
    #[iden = "DELETE_COUNT"]
    DeleteCount,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `PAGE_HASH_THUMBNAIL` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "PAGE_HASH_THUMBNAIL"]
pub enum PageHashThumbnailTable {
    Table,
    #[iden = "HASH"]
    Hash,
    #[iden = "THUMBNAIL"]
    Thumbnail,
}
