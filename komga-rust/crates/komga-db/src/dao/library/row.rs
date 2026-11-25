//! Typed baseline library rows and conversion of file URLs and enum values.

use std::collections::BTreeSet;

use komga_types::{FileUrl, Library, LibraryId, ScanInterval, SeriesCover};
use url::Url;

use crate::DbError;
use crate::support::local_date_time::SqlDateTime;

#[derive(sqlx::FromRow)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) struct LibraryRow {
    pub(super) id: LibraryId,
    name: String,
    root: String,
    #[sqlx(rename = "IMPORT_COMICINFO_BOOK")]
    import_comic_info_book: bool,
    #[sqlx(rename = "IMPORT_COMICINFO_SERIES")]
    import_comic_info_series: bool,
    #[sqlx(rename = "IMPORT_COMICINFO_COLLECTION")]
    import_comic_info_collection: bool,
    #[sqlx(rename = "IMPORT_COMICINFO_READLIST")]
    import_comic_info_read_list: bool,
    #[sqlx(rename = "IMPORT_COMICINFO_SERIES_APPEND_VOLUME")]
    import_comic_info_series_append_volume: bool,
    import_epub_book: bool,
    import_epub_series: bool,
    import_mylar_series: bool,
    import_local_artwork: bool,
    import_barcode_isbn: bool,
    scan_force_modified_time: bool,
    #[sqlx(rename = "SCAN_STARTUP")]
    scan_on_startup: bool,
    scan_interval: ScanInterval,
    scan_cbx: bool,
    scan_pdf: bool,
    scan_epub: bool,
    repair_extensions: bool,
    convert_to_cbz: bool,
    empty_trash_after_scan: bool,
    series_cover: SeriesCover,
    hash_files: bool,
    hash_pages: bool,
    hash_koreader: bool,
    analyze_dimensions: bool,
    oneshots_directory: Option<String>,
    unavailable_date: Option<SqlDateTime>,
}

impl LibraryRow {
    pub(super) fn into_library(self, exclusions: BTreeSet<String>) -> Result<Library, DbError> {
        let url = Url::parse(&self.root).map_err(|error| {
            DbError::UnsupportedSchema(format!("Invalid library ROOT URL: {error}"))
        })?;
        if url.scheme() != "file" {
            return Err(DbError::UnsupportedSchema(
                "Library ROOT is not a local file URL".to_owned(),
            ));
        }
        let root = url
            .to_file_path()
            .map_err(|()| DbError::UnsupportedSchema("Invalid library ROOT path".to_owned()))?;
        Ok(Library {
            id: self.id,
            name: self.name,
            root: root.components().collect(),
            import_comic_info_book: self.import_comic_info_book,
            import_comic_info_series: self.import_comic_info_series,
            import_comic_info_collection: self.import_comic_info_collection,
            import_comic_info_read_list: self.import_comic_info_read_list,
            import_comic_info_series_append_volume: self.import_comic_info_series_append_volume,
            import_epub_book: self.import_epub_book,
            import_epub_series: self.import_epub_series,
            import_mylar_series: self.import_mylar_series,
            import_local_artwork: self.import_local_artwork,
            import_barcode_isbn: self.import_barcode_isbn,
            scan_force_modified_time: self.scan_force_modified_time,
            scan_on_startup: self.scan_on_startup,
            scan_interval: self.scan_interval,
            scan_cbx: self.scan_cbx,
            scan_pdf: self.scan_pdf,
            scan_epub: self.scan_epub,
            scan_directory_exclusions: exclusions,
            repair_extensions: self.repair_extensions,
            convert_to_cbz: self.convert_to_cbz,
            empty_trash_after_scan: self.empty_trash_after_scan,
            series_cover: self.series_cover,
            hash_files: self.hash_files,
            hash_pages: self.hash_pages,
            hash_koreader: self.hash_koreader,
            analyze_dimensions: self.analyze_dimensions,
            oneshots_directory: self.oneshots_directory,
            unavailable_date: self.unavailable_date.map(|date| date.0),
        })
    }
}

pub(super) fn values(
    library: &Library,
) -> [(crate::iden::LibraryTable, sea_query::SimpleExpr); 28] {
    use crate::iden::LibraryTable as L;
    use sea_query::Expr;
    [
        (L::Name, Expr::val(library.name.clone())),
        (
            L::Root,
            Expr::val(FileUrl::directory(&library.root).to_string()),
        ),
        (
            L::ImportComicInfoBook,
            Expr::val(library.import_comic_info_book),
        ),
        (
            L::ImportComicInfoSeries,
            Expr::val(library.import_comic_info_series),
        ),
        (
            L::ImportComicInfoCollection,
            Expr::val(library.import_comic_info_collection),
        ),
        (
            L::ImportComicInfoReadList,
            Expr::val(library.import_comic_info_read_list),
        ),
        (
            L::ImportComicInfoSeriesAppendVolume,
            Expr::val(library.import_comic_info_series_append_volume),
        ),
        (L::ImportEpubBook, Expr::val(library.import_epub_book)),
        (L::ImportEpubSeries, Expr::val(library.import_epub_series)),
        (L::ImportMylarSeries, Expr::val(library.import_mylar_series)),
        (
            L::ImportLocalArtwork,
            Expr::val(library.import_local_artwork),
        ),
        (L::ImportBarcodeIsbn, Expr::val(library.import_barcode_isbn)),
        (
            L::ScanForceModifiedTime,
            Expr::val(library.scan_force_modified_time),
        ),
        (L::ScanOnStartup, Expr::val(library.scan_on_startup)),
        (L::ScanInterval, Expr::val(library.scan_interval)),
        (L::ScanCbx, Expr::val(library.scan_cbx)),
        (L::ScanPdf, Expr::val(library.scan_pdf)),
        (L::ScanEpub, Expr::val(library.scan_epub)),
        (L::RepairExtensions, Expr::val(library.repair_extensions)),
        (L::ConvertToCbz, Expr::val(library.convert_to_cbz)),
        (
            L::EmptyTrashAfterScan,
            Expr::val(library.empty_trash_after_scan),
        ),
        (L::SeriesCover, Expr::val(library.series_cover)),
        (L::HashFiles, Expr::val(library.hash_files)),
        (L::HashPages, Expr::val(library.hash_pages)),
        (L::HashKoreader, Expr::val(library.hash_koreader)),
        (L::AnalyzeDimensions, Expr::val(library.analyze_dimensions)),
        (
            L::OneshotsDirectory,
            Expr::val(library.oneshots_directory.clone()),
        ),
        (
            L::UnavailableDate,
            Expr::val(library.unavailable_date.map(SqlDateTime)),
        ),
    ]
}
