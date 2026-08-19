use komga_application::operational::{
    PageHashAction, PageHashKnownEntry, PageHashMatchEntry, PageHashPage, PageHashUnknownEntry,
};
use komga_interfaces::contracts::page_hashes::{
    known_page_hash_page, page_hash_matches_page, unknown_page_hash_page,
};
use serde_json::json;

#[test]
fn page_hash_contracts_match_kotlin_fields_and_spring_page_shape() {
    let known = known_page_hash_page(&PageHashPage::new(
        0,
        20,
        1,
        vec![PageHashKnownEntry {
            hash: "known-hash".to_string(),
            size: Some(42),
            action: PageHashAction::DeleteAuto,
            delete_count: 2,
            match_count: 3,
            created: "2024-01-02T03:04:05Z".to_string(),
            last_modified: "2024-01-03T03:04:05Z".to_string(),
        }],
        true,
    ))
    .expect("known page hash should map");
    assert_eq!(
        serde_json::to_value(known).expect("known page hash should serialize"),
        json!({
            "content": [{
                "hash": "known-hash",
                "size": 42,
                "action": "DELETE_AUTO",
                "deleteCount": 2,
                "matchCount": 3,
                "created": "2024-01-02T03:04:05",
                "lastModified": "2024-01-03T03:04:05"
            }],
            "pageable": {
                "pageNumber": 0,
                "pageSize": 20,
                "sort": { "empty": false, "sorted": true, "unsorted": false },
                "offset": 0,
                "paged": true,
                "unpaged": false
            },
            "last": true,
            "totalElements": 1,
            "totalPages": 1,
            "first": true,
            "size": 20,
            "number": 0,
            "sort": { "empty": false, "sorted": true, "unsorted": false },
            "numberOfElements": 1,
            "empty": false
        })
    );

    let unknown = unknown_page_hash_page(&PageHashPage::new(
        0,
        20,
        1,
        vec![PageHashUnknownEntry {
            hash: "unknown-hash".to_string(),
            size: None,
            match_count: 1,
        }],
        false,
    ))
    .expect("unknown page hash should map");
    assert_eq!(
        serde_json::to_value(&unknown).expect("unknown page hash should serialize")["content"],
        json!([{ "hash": "unknown-hash", "size": null, "matchCount": 1 }])
    );

    let matches = page_hash_matches_page(&PageHashPage::new(
        0,
        20,
        1,
        vec![PageHashMatchEntry {
            book_id: "book-1".to_string(),
            url: "/library/book.cbz".to_string(),
            page_number: 3,
            file_name: "page-003.jpg".to_string(),
            file_size: 123,
            media_type: "image/jpeg".to_string(),
        }],
        false,
    ))
    .expect("page hash match should map");
    assert_eq!(
        serde_json::to_value(matches).expect("page hash match should serialize")["content"],
        json!([{
            "bookId": "book-1",
            "url": "/library/book.cbz",
            "pageNumber": 3,
            "fileName": "page-003.jpg",
            "fileSize": 123,
            "mediaType": "image/jpeg"
        }])
    );
}
