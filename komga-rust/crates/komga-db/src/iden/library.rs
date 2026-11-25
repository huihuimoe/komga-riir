//! Libraries and the tables their deletion cleans.

use sea_query::Iden;

/// `LIBRARY` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "LIBRARY"]
pub enum LibraryTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "NAME"]
    Name,
    #[iden = "ROOT"]
    Root,
    #[iden = "IMPORT_COMICINFO_BOOK"]
    ImportComicInfoBook,
    #[iden = "IMPORT_COMICINFO_SERIES"]
    ImportComicInfoSeries,
    #[iden = "IMPORT_COMICINFO_COLLECTION"]
    ImportComicInfoCollection,
    #[iden = "IMPORT_COMICINFO_READLIST"]
    ImportComicInfoReadList,
    #[iden = "IMPORT_COMICINFO_SERIES_APPEND_VOLUME"]
    ImportComicInfoSeriesAppendVolume,
    #[iden = "IMPORT_EPUB_BOOK"]
    ImportEpubBook,
    #[iden = "IMPORT_EPUB_SERIES"]
    ImportEpubSeries,
    #[iden = "IMPORT_MYLAR_SERIES"]
    ImportMylarSeries,
    #[iden = "IMPORT_LOCAL_ARTWORK"]
    ImportLocalArtwork,
    #[iden = "IMPORT_BARCODE_ISBN"]
    ImportBarcodeIsbn,
    #[iden = "SCAN_FORCE_MODIFIED_TIME"]
    ScanForceModifiedTime,
    #[iden = "SCAN_STARTUP"]
    ScanOnStartup,
    #[iden = "SCAN_INTERVAL"]
    ScanInterval,
    #[iden = "SCAN_CBX"]
    ScanCbx,
    #[iden = "SCAN_PDF"]
    ScanPdf,
    #[iden = "SCAN_EPUB"]
    ScanEpub,
    #[iden = "REPAIR_EXTENSIONS"]
    RepairExtensions,
    #[iden = "CONVERT_TO_CBZ"]
    ConvertToCbz,
    #[iden = "EMPTY_TRASH_AFTER_SCAN"]
    EmptyTrashAfterScan,
    #[iden = "SERIES_COVER"]
    SeriesCover,
    #[iden = "HASH_FILES"]
    HashFiles,
    #[iden = "HASH_PAGES"]
    HashPages,
    #[iden = "HASH_KOREADER"]
    HashKoreader,
    #[iden = "ANALYZE_DIMENSIONS"]
    AnalyzeDimensions,
    #[iden = "ONESHOTS_DIRECTORY"]
    OneshotsDirectory,
    #[iden = "UNAVAILABLE_DATE"]
    UnavailableDate,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

#[derive(Iden, Clone, Copy)]
#[iden = "LIBRARY_EXCLUSIONS"]
pub enum LibraryExclusionsTable {
    Table,
    #[iden = "LIBRARY_ID"]
    LibraryId,
    #[iden = "EXCLUSION"]
    Exclusion,
}

/// Content tables keyed by book or series, which deletion cleans and restoration moves.
#[derive(Iden, Clone, Copy)]
pub enum LibraryContentTable {
    #[iden = "READ_PROGRESS"]
    ReadProgress,
    #[iden = "READLIST_BOOK"]
    ReadlistBook,
    #[iden = "MEDIA_PAGE"]
    MediaPage,
    #[iden = "MEDIA_FILE"]
    MediaFile,
    #[iden = "MEDIA"]
    Media,
    #[iden = "THUMBNAIL_BOOK"]
    ThumbnailBook,
    #[iden = "BOOK_METADATA_AUTHOR"]
    BookMetadataAuthor,
    #[iden = "BOOK_METADATA_NOTABLE_ENTITIES"]
    BookMetadataNotableEntities,
    #[iden = "BOOK_METADATA_TAG"]
    BookMetadataTag,
    #[iden = "BOOK_METADATA_LINK"]
    BookMetadataLink,
    #[iden = "BOOK_METADATA"]
    BookMetadata,
    #[iden = "BOOK_PROJECTION"]
    BookProjection,
    #[iden = "BOOK"]
    Book,
    #[iden = "READ_PROGRESS_SERIES"]
    ReadProgressSeries,
    #[iden = "COLLECTION_SERIES"]
    CollectionSeries,
    #[iden = "THUMBNAIL_SERIES"]
    ThumbnailSeries,
    #[iden = "SERIES_METADATA_GENRE"]
    SeriesMetadataGenre,
    #[iden = "SERIES_METADATA_TAG"]
    SeriesMetadataTag,
    #[iden = "SERIES_METADATA_SHARING"]
    SeriesMetadataSharing,
    #[iden = "SERIES_METADATA_LINK"]
    SeriesMetadataLink,
    #[iden = "SERIES_METADATA_ALTERNATE_TITLE"]
    SeriesMetadataAlternateTitle,
    #[iden = "SERIES_METADATA"]
    SeriesMetadata,
    #[iden = "BOOK_METADATA_AGGREGATION_AUTHOR"]
    BookMetadataAggregationAuthor,
    #[iden = "BOOK_METADATA_AGGREGATION_NOTABLE_ENTITIES"]
    BookMetadataAggregationNotableEntities,
    #[iden = "BOOK_METADATA_AGGREGATION_TAG"]
    BookMetadataAggregationTag,
    #[iden = "BOOK_METADATA_AGGREGATION"]
    BookMetadataAggregation,
    #[iden = "SERIES"]
    Series,
    #[iden = "SIDECAR"]
    Sidecar,
}

#[derive(Iden, Clone, Copy)]
pub enum ContentKey {
    #[iden = "ID"]
    Id,
    #[iden = "BOOK_ID"]
    BookId,
    #[iden = "SERIES_ID"]
    SeriesId,
    #[iden = "LIBRARY_ID"]
    LibraryId,
    #[iden = "DELETED_DATE"]
    DeletedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}
