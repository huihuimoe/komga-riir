use komga_application::library_catalog::LibraryRecord;

use crate::contracts::library_catalog::LibraryDto;

pub(super) fn libraries_payload(libraries: Vec<LibraryRecord>, is_admin: bool) -> Vec<LibraryDto> {
    libraries
        .iter()
        .map(|library| LibraryDto::from_record(library, is_admin))
        .collect()
}

pub(super) fn library_payload(library: &LibraryRecord, is_admin: bool) -> LibraryDto {
    LibraryDto::from_record(library, is_admin)
}
