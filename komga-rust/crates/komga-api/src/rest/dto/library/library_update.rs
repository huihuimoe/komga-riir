//! Partial library updates, including the deprecated PUT representation.

use std::collections::BTreeSet;
use std::path::PathBuf;

use komga_service::LibraryUpdate;
use komga_types::{ScanInterval, SeriesCover};
use serde::Deserialize;

use crate::error::Violation;
use crate::rest::dto::present;
use crate::rest::validation;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryUpdateDto {
    pub name: Option<String>,
    pub root: Option<String>,
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
    #[serde(default, deserialize_with = "present")]
    pub scan_directory_exclusions: Option<Option<BTreeSet<String>>>,
    pub repair_extensions: Option<bool>,
    pub convert_to_cbz: Option<bool>,
    pub empty_trash_after_scan: Option<bool>,
    pub series_cover: Option<SeriesCover>,
    pub hash_files: Option<bool>,
    pub hash_pages: Option<bool>,
    pub hash_koreader: Option<bool>,
    pub analyze_dimensions: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    pub oneshots_directory: Option<Option<String>>,
}

impl LibraryUpdateDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        for (name, value) in [("name", &self.name), ("root", &self.root)] {
            validation::null_or_not_blank(name, value.as_deref(), &mut violations);
        }
        violations
    }

    pub(crate) fn into_update(self) -> LibraryUpdate {
        LibraryUpdate {
            name: self.name,
            root: self.root.map(PathBuf::from),
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
            scan_directory_exclusions: self
                .scan_directory_exclusions
                .map(Option::unwrap_or_default),
            repair_extensions: self.repair_extensions,
            convert_to_cbz: self.convert_to_cbz,
            empty_trash_after_scan: self.empty_trash_after_scan,
            series_cover: self.series_cover,
            hash_files: self.hash_files,
            hash_pages: self.hash_pages,
            hash_koreader: self.hash_koreader,
            analyze_dimensions: self.analyze_dimensions,
            oneshots_directory: self.oneshots_directory,
        }
    }
}
