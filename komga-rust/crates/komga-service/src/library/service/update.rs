//! Changes to a library's persisted configuration.

use std::collections::BTreeSet;
use std::path::PathBuf;

use komga_types::{Library, ScanInterval, SeriesCover};

#[derive(Debug, Default)]
pub struct LibraryUpdate {
    pub name: Option<String>,
    pub root: Option<PathBuf>,
    pub import_comic_info_book: Option<bool>,
    pub import_comic_info_series: Option<bool>,
    pub import_comic_info_collection: Option<bool>,
    pub import_comic_info_read_list: Option<bool>,
    pub import_comic_info_series_append_volume: Option<bool>,
    pub import_epub_book: Option<bool>,
    pub import_epub_series: Option<bool>,
    pub import_mylar_series: Option<bool>,
    pub import_local_artwork: Option<bool>,
    pub import_barcode_isbn: Option<bool>,
    pub scan_force_modified_time: Option<bool>,
    pub scan_on_startup: Option<bool>,
    pub scan_interval: Option<ScanInterval>,
    pub scan_cbx: Option<bool>,
    pub scan_pdf: Option<bool>,
    pub scan_epub: Option<bool>,
    pub scan_directory_exclusions: Option<BTreeSet<String>>,
    pub repair_extensions: Option<bool>,
    pub convert_to_cbz: Option<bool>,
    pub empty_trash_after_scan: Option<bool>,
    pub series_cover: Option<SeriesCover>,
    pub hash_files: Option<bool>,
    pub hash_pages: Option<bool>,
    pub hash_koreader: Option<bool>,
    pub analyze_dimensions: Option<bool>,
    pub oneshots_directory: Option<Option<String>>,
}

impl LibraryUpdate {
    pub(super) fn into_library(self, existing: Library) -> Library {
        Library {
            name: self.name.unwrap_or(existing.name),
            root: self.root.unwrap_or(existing.root),
            import_comic_info_book: self
                .import_comic_info_book
                .unwrap_or(existing.import_comic_info_book),
            import_comic_info_series: self
                .import_comic_info_series
                .unwrap_or(existing.import_comic_info_series),
            import_comic_info_collection: self
                .import_comic_info_collection
                .unwrap_or(existing.import_comic_info_collection),
            import_comic_info_read_list: self
                .import_comic_info_read_list
                .unwrap_or(existing.import_comic_info_read_list),
            import_comic_info_series_append_volume: self
                .import_comic_info_series_append_volume
                .unwrap_or(existing.import_comic_info_series_append_volume),
            import_epub_book: self.import_epub_book.unwrap_or(existing.import_epub_book),
            import_epub_series: self
                .import_epub_series
                .unwrap_or(existing.import_epub_series),
            import_mylar_series: self
                .import_mylar_series
                .unwrap_or(existing.import_mylar_series),
            import_local_artwork: self
                .import_local_artwork
                .unwrap_or(existing.import_local_artwork),
            import_barcode_isbn: self
                .import_barcode_isbn
                .unwrap_or(existing.import_barcode_isbn),
            scan_force_modified_time: self
                .scan_force_modified_time
                .unwrap_or(existing.scan_force_modified_time),
            scan_on_startup: self.scan_on_startup.unwrap_or(existing.scan_on_startup),
            scan_interval: self.scan_interval.unwrap_or(existing.scan_interval),
            scan_cbx: self.scan_cbx.unwrap_or(existing.scan_cbx),
            scan_pdf: self.scan_pdf.unwrap_or(existing.scan_pdf),
            scan_epub: self.scan_epub.unwrap_or(existing.scan_epub),
            scan_directory_exclusions: self
                .scan_directory_exclusions
                .unwrap_or(existing.scan_directory_exclusions),
            repair_extensions: self.repair_extensions.unwrap_or(existing.repair_extensions),
            convert_to_cbz: self.convert_to_cbz.unwrap_or(existing.convert_to_cbz),
            empty_trash_after_scan: self
                .empty_trash_after_scan
                .unwrap_or(existing.empty_trash_after_scan),
            series_cover: self.series_cover.unwrap_or(existing.series_cover),
            hash_files: self.hash_files.unwrap_or(existing.hash_files),
            hash_pages: self.hash_pages.unwrap_or(existing.hash_pages),
            hash_koreader: self.hash_koreader.unwrap_or(existing.hash_koreader),
            analyze_dimensions: self
                .analyze_dimensions
                .unwrap_or(existing.analyze_dimensions),
            oneshots_directory: self
                .oneshots_directory
                .unwrap_or(existing.oneshots_directory),
            ..existing
        }
    }
}
