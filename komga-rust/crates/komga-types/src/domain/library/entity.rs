//! Library configuration and filesystem identity from the Kotlin Library aggregate.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::ids::LibraryId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    pub id: LibraryId,
    pub name: String,
    pub root: PathBuf,
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
    /// Server local time of the last scan that found the root inaccessible.
    pub unavailable_date: Option<jiff::civil::DateTime>,
}

impl Library {
    #[must_use]
    pub fn new(name: String, root: PathBuf) -> Self {
        Self {
            id: LibraryId::generate(),
            name,
            root,
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
            unavailable_date: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum ScanInterval {
    #[serde(rename = "DISABLED")]
    Disabled,
    #[serde(rename = "HOURLY")]
    Hourly,
    #[serde(rename = "EVERY_6H")]
    Every6h,
    #[serde(rename = "EVERY_12H")]
    Every12h,
    #[serde(rename = "DAILY")]
    Daily,
    #[serde(rename = "WEEKLY")]
    Weekly,
}

impl ScanInterval {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "DISABLED",
            Self::Hourly => "HOURLY",
            Self::Every6h => "EVERY_6H",
            Self::Every12h => "EVERY_12H",
            Self::Daily => "DAILY",
            Self::Weekly => "WEEKLY",
        }
    }

    /// The time between periodic scans; `None` when they are disabled.
    #[must_use]
    pub fn period(self) -> Option<Duration> {
        const HOUR: u64 = 60 * 60;
        let hours = match self {
            Self::Disabled => return None,
            Self::Hourly => 1,
            Self::Every6h => 6,
            Self::Every12h => 12,
            Self::Daily => 24,
            Self::Weekly => 7 * 24,
        };
        Some(Duration::from_secs(hours * HOUR))
    }
}

impl FromStr for ScanInterval {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "DISABLED" => Ok(Self::Disabled),
            "HOURLY" => Ok(Self::Hourly),
            "EVERY_6H" => Ok(Self::Every6h),
            "EVERY_12H" => Ok(Self::Every12h),
            "DAILY" => Ok(Self::Daily),
            "WEEKLY" => Ok(Self::Weekly),
            _ => Err("Invalid ScanInterval"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum SeriesCover {
    #[serde(rename = "FIRST")]
    First,
    #[serde(rename = "FIRST_UNREAD_OR_FIRST")]
    FirstUnreadOrFirst,
    #[serde(rename = "FIRST_UNREAD_OR_LAST")]
    FirstUnreadOrLast,
    #[serde(rename = "LAST")]
    Last,
}

impl SeriesCover {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::First => "FIRST",
            Self::FirstUnreadOrFirst => "FIRST_UNREAD_OR_FIRST",
            Self::FirstUnreadOrLast => "FIRST_UNREAD_OR_LAST",
            Self::Last => "LAST",
        }
    }
}

impl FromStr for SeriesCover {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "FIRST" => Ok(Self::First),
            "FIRST_UNREAD_OR_FIRST" => Ok(Self::FirstUnreadOrFirst),
            "FIRST_UNREAD_OR_LAST" => Ok(Self::FirstUnreadOrLast),
            "LAST" => Ok(Self::Last),
            _ => Err("Invalid SeriesCover"),
        }
    }
}

crate::sql::sql_enum!(ScanInterval => as_str, SeriesCover => as_str);
