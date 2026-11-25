use std::io::Read;

use flate2::read::GzDecoder;
use komga_test_support::TestDir;
use komga_types::{
    BookId, BookPage, Dimension, EpubTocEntry, Media, MediaExtensionEpub, MediaFile,
    MediaFileSubType, MediaStatus, R2Location, R2Locator, R2Text,
};
use serde_json::{Value, json};

use super::MediaDao;
use crate::{Database, DbTransaction};

/// A book `b` with an `UNKNOWN` media.
async fn database_with_book() -> (TestDir, Database, BookId) {
    let directory = komga_test_support::tempdir().expect("Database directory");
    let db = Database::connect(&directory.path().join("database.sqlite"))
        .await
        .expect("Open database");
    db.migrate().await.expect("Migrate database");
    let mut tx = db.begin().await.expect("Begin");
    for statement in [
        "INSERT INTO LIBRARY (ID, NAME, ROOT) VALUES ('l', 'L', 'file:/l/')",
        "INSERT INTO SERIES (ID, FILE_LAST_MODIFIED, NAME, URL, LIBRARY_ID) \
         VALUES ('s', CURRENT_TIMESTAMP, 'S', 'file:/l/s/', 'l')",
        "INSERT INTO BOOK (ID, FILE_LAST_MODIFIED, NAME, URL, NUMBER, SERIES_ID, LIBRARY_ID) \
         VALUES ('b', CURRENT_TIMESTAMP, 'b', 'file:/l/s/b.cbz', 1, 's', 'l')",
    ] {
        sqlx::query(statement)
            .execute(tx.connection())
            .await
            .expect("Fixture");
    }
    let book = BookId::from("b".to_owned());
    MediaDao::insert_unknown(&mut tx, std::slice::from_ref(&book))
        .await
        .expect("Insert media");
    tx.commit().await.expect("Commit");
    (directory, db, book)
}

#[tokio::test]
async fn an_updated_media_reads_back_with_its_pages_and_files() {
    let (_directory, db, book) = database_with_book().await;
    let mut tx = db.begin().await.expect("Begin");
    let page = |name: &str, dimension| BookPage {
        file_name: name.to_owned(),
        media_type: "image/png".to_owned(),
        dimension,
        file_hash: String::new(),
        file_size: Some(7),
    };
    let media = Media {
        status: MediaStatus::Ready,
        media_type: Some("application/zip".to_owned()),
        pages: vec![
            page(
                "2.png",
                Some(Dimension {
                    width: 3,
                    height: 2,
                }),
            ),
            page("1.png", None),
        ],
        page_count: 2,
        files: vec![MediaFile {
            file_name: "ComicInfo.xml".to_owned(),
            media_type: None,
            sub_type: None,
            file_size: None,
        }],
        comment: Some("ERR_1007 [x]".to_owned()),
        epub_divina_compatible: false,
        epub_is_kepub: false,
        extension: None,
    };
    for _ in 0..2 {
        MediaDao::update(&mut tx, &book, &media)
            .await
            .expect("Update media");
    }
    assert_eq!(
        MediaDao::find_by_id(&mut tx, &book)
            .await
            .expect("Find media"),
        Some(media),
        "pages keep their order, and updating again replaces pages and files"
    );
}

async fn stored_extension(
    tx: &mut DbTransaction,
    book: &BookId,
) -> (Option<String>, Option<Vec<u8>>) {
    sqlx::query_as("SELECT EXTENSION_CLASS, EXTENSION_VALUE_BLOB FROM MEDIA WHERE BOOK_ID = ?")
        .bind(book.as_str())
        .fetch_one(tx.connection())
        .await
        .expect("Stored extension")
}

/// A ready KEPUB with sub-typed files and an extension that exercises every inclusion rule.
fn epub_media() -> Media {
    let file = |name: &str, sub_type| MediaFile {
        file_name: name.to_owned(),
        media_type: Some("application/xhtml+xml".to_owned()),
        sub_type,
        file_size: Some(3),
    };
    Media {
        status: MediaStatus::Ready,
        media_type: Some("application/epub+zip".to_owned()),
        pages: Vec::new(),
        page_count: 4,
        files: vec![
            file("OEBPS/c1.xhtml", Some(MediaFileSubType::EpubPage)),
            file("OEBPS/style.css", Some(MediaFileSubType::EpubAsset)),
            file("mimetype", None),
        ],
        comment: None,
        epub_divina_compatible: false,
        epub_is_kepub: true,
        extension: Some(MediaExtensionEpub {
            toc: vec![EpubTocEntry {
                title: "Part".to_owned(),
                href: None,
                children: vec![EpubTocEntry {
                    title: "One".to_owned(),
                    href: Some("OEBPS/c1.xhtml#a".to_owned()),
                    children: Vec::new(),
                }],
            }],
            landmarks: Vec::new(),
            page_list: Vec::new(),
            is_fixed_layout: false,
            positions: vec![
                R2Locator {
                    href: "OEBPS/c1.xhtml".to_owned(),
                    media_type: "application/xhtml+xml".to_owned(),
                    title: Some(String::new()),
                    locations: Some(R2Location {
                        fragments: Vec::new(),
                        progression: Some(0.0),
                        position: Some(1),
                        total_progression: Some(0.25),
                    }),
                    text: None,
                    kobo_span: Some("kobo.1.1".to_owned()),
                },
                R2Locator {
                    href: "OEBPS/c1.xhtml".to_owned(),
                    media_type: "application/xhtml+xml".to_owned(),
                    title: Some("One".to_owned()),
                    locations: Some(R2Location {
                        fragments: vec!["a".to_owned()],
                        ..R2Location::default()
                    }),
                    text: Some(R2Text {
                        highlight: Some("x".to_owned()),
                        ..R2Text::default()
                    }),
                    kobo_span: None,
                },
            ],
        }),
    }
}

#[tokio::test]
async fn an_epub_extension_is_stored_as_kotlin_gzipped_json_and_kept_by_other_updates() {
    let (_directory, db, book) = database_with_book().await;
    let mut tx = db.begin().await.expect("Begin");
    let epub = epub_media();
    MediaDao::update(&mut tx, &book, &epub)
        .await
        .expect("Update media");

    let (class, blob) = stored_extension(&mut tx, &book).await;
    assert_eq!(
        class.as_deref(),
        Some("org.gotson.komga.domain.model.MediaExtensionEpub")
    );
    let blob = blob.expect("Extension blob");
    let mut json = String::new();
    GzDecoder::new(blob.as_slice())
        .read_to_string(&mut json)
        .expect("The blob is a gzip member");
    let json: Value = serde_json::from_str(&json).expect("The blob holds JSON");
    assert_eq!(
        json,
        json!({
            "toc": [{
                "title": "Part",
                "href": null,
                "children": [{"title": "One", "href": "OEBPS/c1.xhtml#a", "children": []}],
            }],
            "landmarks": [],
            "pageList": [],
            "isFixedLayout": false,
            "positions": [
                {
                    "href": "OEBPS/c1.xhtml",
                    "type": "application/xhtml+xml",
                    "locations": {"progression": 0.0, "position": 1, "totalProgression": 0.25},
                    "koboSpan": "kobo.1.1",
                },
                {
                    "href": "OEBPS/c1.xhtml",
                    "type": "application/xhtml+xml",
                    "title": "One",
                    "locations": {"fragments": ["a"]},
                    "text": {"highlight": "x"},
                },
            ],
        }),
        "the extension keeps Jackson's nulls, and locators are NON_EMPTY"
    );

    let stored = MediaDao::find_by_id(&mut tx, &book)
        .await
        .expect("Find media")
        .expect("Media");
    assert_eq!(stored.files, epub.files, "file sub types round-trip");

    let failed = Media::failed(MediaStatus::Error, Some("ERR_1005".to_owned()));
    MediaDao::update(&mut tx, &book, &failed)
        .await
        .expect("Update media");
    assert_eq!(
        stored_extension(&mut tx, &book).await,
        (class, Some(blob)),
        "a media without extension keeps the stored one"
    );
}

#[tokio::test]
async fn a_stored_epub_extension_reads_back_unless_absent_or_undecodable() {
    let (_directory, db, book) = database_with_book().await;
    let mut tx = db.begin().await.expect("Begin");
    assert_eq!(
        MediaDao::find_extension(&mut tx, &book)
            .await
            .expect("Find extension"),
        None,
        "nothing stored"
    );

    let epub = epub_media();
    MediaDao::update(&mut tx, &book, &epub)
        .await
        .expect("Update media");
    let mut expected = epub.extension.expect("Extension");
    // NON_EMPTY dropped the empty title.
    expected.positions[0].title = None;
    assert_eq!(
        MediaDao::find_extension(&mut tx, &book)
            .await
            .expect("Find extension"),
        Some(expected)
    );

    sqlx::query("UPDATE MEDIA SET EXTENSION_VALUE_BLOB = X'00' WHERE BOOK_ID = ?")
        .bind(book.as_str())
        .execute(tx.connection())
        .await
        .expect("Corrupt the blob");
    assert_eq!(
        MediaDao::find_extension(&mut tx, &book)
            .await
            .expect("Find extension"),
        None,
        "an undecodable blob"
    );
}

#[tokio::test]
async fn books_missing_page_hashes_lack_one_of_the_first_and_last_pages() {
    let (_directory, db, book) = database_with_book().await;
    let mut tx = db.begin().await.expect("Begin");
    let media = |page_count: usize, hashed: usize, status| Media {
        status,
        media_type: Some("application/zip".to_owned()),
        pages: (0..page_count)
            .map(|index| BookPage {
                file_name: format!("{index}.jpg"),
                media_type: "image/jpeg".to_owned(),
                dimension: None,
                file_hash: if index < hashed {
                    format!("h{index}")
                } else {
                    String::new()
                },
                file_size: None,
            })
            .collect(),
        page_count: i32::try_from(page_count).expect("Few pages"),
        files: Vec::new(),
        comment: None,
        epub_divina_compatible: false,
        epub_is_kepub: false,
        extension: None,
    };
    for (media, missing, case) in [
        (media(10, 5, MediaStatus::Ready), true, "5 of 6 hashed"),
        (media(10, 6, MediaStatus::Ready), false, "6 of 6 hashed"),
        (media(4, 3, MediaStatus::Ready), true, "3 of 4 pages hashed"),
        (media(4, 4, MediaStatus::Ready), false, "all 4 pages hashed"),
        (media(0, 0, MediaStatus::Ready), false, "no pages"),
        (media(10, 0, MediaStatus::Error), false, "not ready"),
    ] {
        MediaDao::update(&mut tx, &book, &media)
            .await
            .expect("Update media");
        let library = "l".to_owned().into();
        let found = MediaDao::find_book_ids_with_missing_page_hash(
            &mut tx,
            &library,
            &["application/zip"],
            3,
        )
        .await
        .expect("Find books");
        assert_eq!(found == [book.clone()], missing, "{case}");
        let other_type = MediaDao::find_book_ids_with_missing_page_hash(
            &mut tx,
            &library,
            &["application/pdf"],
            3,
        )
        .await
        .expect("Find books");
        assert!(other_type.is_empty(), "{case}: another media type");
    }
}
