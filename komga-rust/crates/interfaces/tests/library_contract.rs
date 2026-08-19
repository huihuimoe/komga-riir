use komga_application::library_catalog::LibraryRecord;
use komga_interfaces::contracts::library_catalog::LibraryDto;
use serde_json::json;

#[test]
fn library_dto_matches_kotlin_field_shape_and_restricts_root() {
    let mut library = LibraryRecord::default_record("library-1".to_string());
    library.name = "Library".to_string();
    library.root = "file:///data/library".to_string();
    library.scan_directory_exclusions = vec!["tmp".to_string()];
    library.oneshots_directory = Some("oneshots".to_string());
    library.unavailable = true;

    let payload = serde_json::to_value(LibraryDto::from_record(&library, true))
        .expect("library should serialize");

    assert_eq!(
        payload
            .as_object()
            .expect("library should be an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            "analyzeDimensions",
            "convertToCbz",
            "emptyTrashAfterScan",
            "hashFiles",
            "hashKoreader",
            "hashPages",
            "id",
            "importBarcodeIsbn",
            "importComicInfoBook",
            "importComicInfoCollection",
            "importComicInfoReadList",
            "importComicInfoSeries",
            "importComicInfoSeriesAppendVolume",
            "importEpubBook",
            "importEpubSeries",
            "importLocalArtwork",
            "importMylarSeries",
            "name",
            "oneshotsDirectory",
            "repairExtensions",
            "root",
            "scanCbx",
            "scanDirectoryExclusions",
            "scanEpub",
            "scanForceModifiedTime",
            "scanInterval",
            "scanOnStartup",
            "scanPdf",
            "seriesCover",
            "unavailable",
        ]
    );
    assert_eq!(payload["root"], json!("/data/library"));
    assert_eq!(payload["scanInterval"], json!("EVERY_6H"));
    assert_eq!(payload["seriesCover"], json!("FIRST"));
    assert_eq!(payload["scanDirectoryExclusions"], json!(["tmp"]));
    assert_eq!(payload["unavailable"], json!(true));

    let restricted = serde_json::to_value(LibraryDto::from_record(&library, false))
        .expect("library should serialize");
    assert_eq!(restricted["root"], json!(""));
}
