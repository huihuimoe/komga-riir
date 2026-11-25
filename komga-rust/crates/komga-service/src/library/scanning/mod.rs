//! Library content scanning and trash recovery.

mod file_system_scanner;
mod ignore_case;
mod library_content_service;
mod library_scan;
pub(crate) mod sidecar_names;

pub use file_system_scanner::ScanError;
pub(crate) use file_system_scanner::{scan_file, scan_root_folder};
pub use library_content_service::{LibraryContentError, LibraryContentService};
