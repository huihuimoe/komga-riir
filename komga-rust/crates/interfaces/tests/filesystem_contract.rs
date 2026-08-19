use komga_interfaces::contracts::filesystem::{
    DirectoryListingDto, FilesystemEntryDto, FilesystemEntryTypeDto,
};
use serde_json::json;

#[test]
fn filesystem_dto_matches_kotlin_field_shape_and_omits_empty_parent() {
    let payload = serde_json::to_value(DirectoryListingDto {
        parent: None,
        directories: vec![FilesystemEntryDto {
            entry_type: FilesystemEntryTypeDto::Directory,
            name: "library".to_string(),
            path: "/library".to_string(),
        }],
        files: vec![FilesystemEntryDto {
            entry_type: FilesystemEntryTypeDto::File,
            name: "book.cbz".to_string(),
            path: "/book.cbz".to_string(),
        }],
    })
    .expect("filesystem listing should serialize");

    assert_eq!(
        payload,
        json!({
            "directories": [{ "type": "directory", "name": "library", "path": "/library" }],
            "files": [{ "type": "file", "name": "book.cbz", "path": "/book.cbz" }]
        })
    );
}
