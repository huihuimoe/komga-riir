use std::collections::HashSet;
use std::path::PathBuf;

use komga_types::{BookPage, Media, MediaFile, MediaStatus};

use super::ConversionError;
use super::cbz::{verify, write_cbz};

fn page(name: &str) -> BookPage {
    BookPage {
        file_name: name.to_owned(),
        media_type: "image/png".to_owned(),
        dimension: None,
        file_hash: String::new(),
        file_size: None,
    }
}

fn media(media_type: &str, pages: &[&str], files: &[&str]) -> Media {
    Media {
        status: MediaStatus::Ready,
        media_type: Some(media_type.to_owned()),
        pages: pages.iter().map(|name| page(name)).collect(),
        page_count: i32::try_from(pages.len()).expect("Few pages"),
        files: files
            .iter()
            .map(|name| MediaFile {
                file_name: (*name).to_owned(),
                media_type: None,
                sub_type: None,
                file_size: None,
            })
            .collect(),
        comment: None,
        epub_divina_compatible: false,
        epub_is_kepub: false,
        extension: None,
    }
}

fn rar(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../komga-archive/tests/fixtures/rar")
        .join(name)
}

#[test]
fn a_converted_file_must_be_a_ready_zip_holding_every_page_and_file() {
    let original = media(
        super::CONVERTIBLE_TYPES[1],
        &["a/1.png", "2.png"],
        &["info.txt"],
    );
    let unverified = |converted: &Media| {
        matches!(
            verify(&original, converted),
            Err(ConversionError::Unverified(_))
        )
    };

    // Directories may go; base names and page types must stay.
    assert!(
        verify(
            &original,
            &media(super::ZIP, &["1.png", "2.png", "3.png"], &["info.txt"])
        )
        .is_ok()
    );
    assert!(unverified(&media(super::ZIP, &["1.png"], &["info.txt"])));
    assert!(unverified(&media(super::ZIP, &["1.png", "2.png"], &[])));
    assert!(unverified(&media(
        "application/pdf",
        &["1.png", "2.png"],
        &["info.txt"]
    )));
    let mut failed = media(super::ZIP, &["1.png", "2.png"], &["info.txt"]);
    failed.status = MediaStatus::Error;
    assert!(unverified(&failed));
    let mut retyped = media(super::ZIP, &["1.png", "2.png"], &["info.txt"]);
    retyped.pages[0].media_type = "image/jpeg".to_owned();
    assert!(unverified(&retyped));
}

#[test]
fn a_member_missing_from_the_rar_file_fails_the_conversion() {
    let directory = komga_test_support::tempdir().expect("Directory");
    let target = directory.path().join("book.cbz");
    let present: HashSet<String> = ["komga-1.png", "komga-2.png"].map(str::to_owned).into();
    write_cbz(&rar("rar4-solid.rar"), &target, &present).expect("Present members");

    let mut missing = present;
    missing.insert("komga-9.png".to_owned());
    let result = write_cbz(&rar("rar4-solid.rar"), &target, &missing);
    assert!(
        matches!(result, Err(ConversionError::Unverified(_))),
        "{result:?}"
    );
}
