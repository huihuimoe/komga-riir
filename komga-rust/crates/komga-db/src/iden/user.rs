//! Users, their settings, reading state, sharing and sync points.

use sea_query::Iden;

/// `USER` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "USER"]
pub enum UserTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
    #[iden = "EMAIL"]
    Email,
    #[iden = "PASSWORD"]
    Password,
    #[iden = "SHARED_ALL_LIBRARIES"]
    SharedAllLibraries,
    #[iden = "AGE_RESTRICTION"]
    AgeRestriction,
    #[iden = "AGE_RESTRICTION_ALLOW_ONLY"]
    AgeRestrictionAllowOnly,
}

/// `USER_ROLE` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "USER_ROLE"]
pub enum UserRoleTable {
    Table,
    #[iden = "USER_ID"]
    UserId,
    #[iden = "ROLE"]
    Role,
}

/// `USER_API_KEY` table column identifiers matching baseline Flyway schema.
#[derive(Iden, Clone, Copy)]
#[iden = "USER_API_KEY"]
pub enum UserApiKeyTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "USER_ID"]
    UserId,
    #[iden = "API_KEY"]
    ApiKey,
    #[iden = "COMMENT"]
    Comment,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `AUTHENTICATION_ACTIVITY` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "AUTHENTICATION_ACTIVITY"]
pub enum AuthenticationActivityTable {
    Table,
    #[iden = "USER_ID"]
    UserId,
    #[iden = "EMAIL"]
    Email,
    #[iden = "IP"]
    Ip,
    #[iden = "USER_AGENT"]
    UserAgent,
    #[iden = "SUCCESS"]
    Success,
    #[iden = "ERROR"]
    Error,
    #[iden = "DATE_TIME"]
    DateTime,
    #[iden = "SOURCE"]
    Source,
    #[iden = "API_KEY_ID"]
    ApiKeyId,
    #[iden = "API_KEY_COMMENT"]
    ApiKeyComment,
}

/// `CLIENT_SETTINGS_GLOBAL` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "CLIENT_SETTINGS_GLOBAL"]
pub enum ClientSettingsGlobalTable {
    Table,
    #[iden = "KEY"]
    Key,
    #[iden = "VALUE"]
    Value,
    #[iden = "ALLOW_UNAUTHORIZED"]
    AllowUnauthorized,
}

/// `CLIENT_SETTINGS_USER` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "CLIENT_SETTINGS_USER"]
pub enum ClientSettingsUserTable {
    Table,
    #[iden = "USER_ID"]
    UserId,
    #[iden = "KEY"]
    Key,
    #[iden = "VALUE"]
    Value,
}

/// `READ_PROGRESS` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "READ_PROGRESS"]
pub enum ReadProgressTable {
    Table,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "USER_ID"]
    UserId,
    #[iden = "PAGE"]
    Page,
    #[iden = "COMPLETED"]
    Completed,
    #[iden = "READ_DATE"]
    ReadDate,
    #[iden = "DEVICE_ID"]
    DeviceId,
    #[iden = "DEVICE_NAME"]
    DeviceName,
    #[iden = "LOCATOR"]
    Locator,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `READ_PROGRESS_SERIES` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "READ_PROGRESS_SERIES"]
pub enum ReadProgressSeriesTable {
    Table,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "USER_ID"]
    UserId,
    #[iden = "READ_COUNT"]
    ReadCount,
    #[iden = "IN_PROGRESS_COUNT"]
    InProgressCount,
    #[iden = "MOST_RECENT_READ_DATE"]
    MostRecentReadDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// `USER_LIBRARY_SHARING` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "USER_LIBRARY_SHARING"]
pub enum UserLibrarySharingTable {
    Table,
    #[iden = "USER_ID"]
    UserId,
    #[iden = "LIBRARY_ID"]
    LibraryId,
}

/// `USER_SHARING` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "USER_SHARING"]
pub enum UserSharingTable {
    Table,
    #[iden = "USER_ID"]
    UserId,
    #[iden = "LABEL"]
    Label,
    #[iden = "ALLOW"]
    Allow,
}

/// `ANNOUNCEMENTS_READ` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "ANNOUNCEMENTS_READ"]
pub enum AnnouncementsReadTable {
    Table,
    #[iden = "USER_ID"]
    UserId,
    #[iden = "ANNOUNCEMENT_ID"]
    AnnouncementId,
}

/// `SYNC_POINT` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SYNC_POINT"]
pub enum SyncPointTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "USER_ID"]
    UserId,
    #[iden = "API_KEY_ID"]
    ApiKeyId,
    #[iden = "CREATED_DATE"]
    CreatedDate,
}

/// `SYNC_POINT_BOOK` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SYNC_POINT_BOOK"]
pub enum SyncPointBookTable {
    Table,
    #[iden = "SYNC_POINT_ID"]
    SyncPointId,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "BOOK_CREATED_DATE"]
    BookCreatedDate,
    #[iden = "BOOK_LAST_MODIFIED_DATE"]
    BookLastModifiedDate,
    #[iden = "BOOK_FILE_LAST_MODIFIED"]
    BookFileLastModified,
    #[iden = "BOOK_FILE_SIZE"]
    BookFileSize,
    #[iden = "BOOK_FILE_HASH"]
    BookFileHash,
    #[iden = "BOOK_METADATA_LAST_MODIFIED_DATE"]
    BookMetadataLastModifiedDate,
    #[iden = "BOOK_READ_PROGRESS_LAST_MODIFIED_DATE"]
    BookReadProgressLastModifiedDate,
    #[iden = "BOOK_THUMBNAIL_ID"]
    BookThumbnailId,
    #[iden = "SYNCED"]
    Synced,
}

/// `SYNC_POINT_BOOK_REMOVED_SYNCED` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SYNC_POINT_BOOK_REMOVED_SYNCED"]
pub enum SyncPointBookRemovedSyncedTable {
    Table,
    #[iden = "SYNC_POINT_ID"]
    SyncPointId,
    #[iden = "BOOK_ID"]
    BookId,
}

/// `SYNC_POINT_READLIST` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SYNC_POINT_READLIST"]
pub enum SyncPointReadListTable {
    Table,
    #[iden = "SYNC_POINT_ID"]
    SyncPointId,
    #[iden = "READLIST_ID"]
    ReadListId,
    #[iden = "READLIST_NAME"]
    ReadListName,
    #[iden = "READLIST_CREATED_DATE"]
    ReadListCreatedDate,
    #[iden = "READLIST_LAST_MODIFIED_DATE"]
    ReadListLastModifiedDate,
    #[iden = "SYNCED"]
    Synced,
}

/// `SYNC_POINT_READLIST_BOOK` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SYNC_POINT_READLIST_BOOK"]
pub enum SyncPointReadListBookTable {
    Table,
    #[iden = "SYNC_POINT_ID"]
    SyncPointId,
    #[iden = "READLIST_ID"]
    ReadListId,
    #[iden = "BOOK_ID"]
    BookId,
}

/// `SYNC_POINT_READLIST_REMOVED_SYNCED` table identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SYNC_POINT_READLIST_REMOVED_SYNCED"]
pub enum SyncPointReadListRemovedSyncedTable {
    Table,
    #[iden = "SYNC_POINT_ID"]
    SyncPointId,
    #[iden = "READLIST_ID"]
    ReadListId,
}
