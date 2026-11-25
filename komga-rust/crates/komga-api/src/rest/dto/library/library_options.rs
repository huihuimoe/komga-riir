//! Library option DTOs, shared by creation and representation.

use std::collections::BTreeSet;

use komga_types::{Library, ScanInterval, SeriesCover};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LibraryOptionsDto {
    pub import_comic_info_book: bool,
    pub import_comic_info_series: bool,
    pub import_comic_info_collection: bool,
    pub import_comic_info_read_list: bool,
    pub import_comic_info_series_append_volume: bool,
    pub import_epub_book: bool,
    pub import_epub_series: bool,
    pub import_mylar_series: bool,
    pub import_local_artwork: bool,
    pub import_barcode_isbn: bool,
    pub scan_force_modified_time: bool,
    pub scan_on_startup: bool,
    pub scan_interval: ScanInterval,
    pub scan_cbx: bool,
    pub scan_pdf: bool,
    pub scan_epub: bool,
    pub scan_directory_exclusions: BTreeSet<String>,
    pub repair_extensions: bool,
    pub convert_to_cbz: bool,
    pub empty_trash_after_scan: bool,
    pub series_cover: SeriesCover,
    pub hash_files: bool,
    pub hash_pages: bool,
    pub hash_koreader: bool,
    pub analyze_dimensions: bool,
    pub oneshots_directory: Option<String>,
}

impl Default for LibraryOptionsDto {
    fn default() -> Self {
        Self {
            import_comic_info_book: true,
            import_comic_info_series: true,
            import_comic_info_collection: true,
            import_comic_info_read_list: true,
            import_comic_info_series_append_volume: true,
            import_epub_book: true,
            import_epub_series: true,
            import_mylar_series: true,
            import_local_artwork: true,
            import_barcode_isbn: true,
            scan_force_modified_time: false,
            scan_on_startup: false,
            scan_interval: ScanInterval::Every6h,
            scan_cbx: true,
            scan_pdf: true,
            scan_epub: true,
            scan_directory_exclusions: BTreeSet::new(),
            repair_extensions: false,
            convert_to_cbz: false,
            empty_trash_after_scan: false,
            series_cover: SeriesCover::First,
            hash_files: true,
            hash_pages: false,
            hash_koreader: false,
            analyze_dimensions: true,
            oneshots_directory: None,
        }
    }
}

impl LibraryOptionsDto {
    pub(super) fn apply(self, library: &mut Library) {
        library.import_comic_info_book = self.import_comic_info_book;
        library.import_comic_info_series = self.import_comic_info_series;
        library.import_comic_info_collection = self.import_comic_info_collection;
        library.import_comic_info_read_list = self.import_comic_info_read_list;
        library.import_comic_info_series_append_volume =
            self.import_comic_info_series_append_volume;
        library.import_epub_book = self.import_epub_book;
        library.import_epub_series = self.import_epub_series;
        library.import_mylar_series = self.import_mylar_series;
        library.import_local_artwork = self.import_local_artwork;
        library.import_barcode_isbn = self.import_barcode_isbn;
        library.scan_force_modified_time = self.scan_force_modified_time;
        library.scan_on_startup = self.scan_on_startup;
        library.scan_interval = self.scan_interval;
        library.scan_cbx = self.scan_cbx;
        library.scan_pdf = self.scan_pdf;
        library.scan_epub = self.scan_epub;
        library.scan_directory_exclusions = self.scan_directory_exclusions;
        library.repair_extensions = self.repair_extensions;
        library.convert_to_cbz = self.convert_to_cbz;
        library.empty_trash_after_scan = self.empty_trash_after_scan;
        library.series_cover = self.series_cover;
        library.hash_files = self.hash_files;
        library.hash_pages = self.hash_pages;
        library.hash_koreader = self.hash_koreader;
        library.analyze_dimensions = self.analyze_dimensions;
        library.oneshots_directory = self.oneshots_directory;
    }
}

impl From<Library> for LibraryOptionsDto {
    fn from(library: Library) -> Self {
        Self {
            import_comic_info_book: library.import_comic_info_book,
            import_comic_info_series: library.import_comic_info_series,
            import_comic_info_collection: library.import_comic_info_collection,
            import_comic_info_read_list: library.import_comic_info_read_list,
            import_comic_info_series_append_volume: library.import_comic_info_series_append_volume,
            import_epub_book: library.import_epub_book,
            import_epub_series: library.import_epub_series,
            import_mylar_series: library.import_mylar_series,
            import_local_artwork: library.import_local_artwork,
            import_barcode_isbn: library.import_barcode_isbn,
            scan_force_modified_time: library.scan_force_modified_time,
            scan_on_startup: library.scan_on_startup,
            scan_interval: library.scan_interval,
            scan_cbx: library.scan_cbx,
            scan_pdf: library.scan_pdf,
            scan_epub: library.scan_epub,
            scan_directory_exclusions: library.scan_directory_exclusions,
            repair_extensions: library.repair_extensions,
            convert_to_cbz: library.convert_to_cbz,
            empty_trash_after_scan: library.empty_trash_after_scan,
            series_cover: library.series_cover,
            hash_files: library.hash_files,
            hash_pages: library.hash_pages,
            hash_koreader: library.hash_koreader,
            analyze_dimensions: library.analyze_dimensions,
            oneshots_directory: library.oneshots_directory,
        }
    }
}
