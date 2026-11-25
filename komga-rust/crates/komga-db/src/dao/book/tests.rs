use komga_types::{Book, LibraryId};

use super::{BookDao, BookHash};
use crate::{Database, DbTransaction};

fn ids(books: &[Book]) -> Vec<&str> {
    books.iter().map(|book| book.id.as_str()).collect()
}

/// Library `l` holds `b1`–`b4`; `bo` matches every query but belongs to library `o`.
async fn insert_books(tx: &mut DbTransaction) {
    for statement in [
        "INSERT INTO LIBRARY (ID, NAME, ROOT) VALUES ('l', 'L', 'file:/l/'), ('o', 'O', 'file:/o/')",
        "INSERT INTO SERIES (ID, FILE_LAST_MODIFIED, NAME, URL, LIBRARY_ID) VALUES \
         ('s1', CURRENT_TIMESTAMP, 'S1', 'file:/l/s1/', 'l'), \
         ('s2', CURRENT_TIMESTAMP, 'S2', 'file:/l/s2/', 'l'), \
         ('so', CURRENT_TIMESTAMP, 'SO', 'file:/o/so/', 'o')",
        "INSERT INTO BOOK (ID, FILE_LAST_MODIFIED, NAME, URL, NUMBER, FILE_HASH, \
         FILE_HASH_KOREADER, SERIES_ID, LIBRARY_ID) VALUES \
         ('b1', CURRENT_TIMESTAMP, 'b1', 'file:/l/s2/b1.CBZ', 1, '', 'k', 's2', 'l'), \
         ('b2', CURRENT_TIMESTAMP, 'b2', 'file:/l/s1/b2.cbr', 2, 'h', '', 's1', 'l'), \
         ('b3', CURRENT_TIMESTAMP, 'b3', 'file:/l/s1/b3.cbz', 1, 'h', 'k', 's1', 'l'), \
         ('b4', CURRENT_TIMESTAMP, 'b4', 'file:/l/s1/b4.cbz', 3, 'h', 'k', 's1', 'l'), \
         ('bo', CURRENT_TIMESTAMP, 'bo', 'file:/o/so/bo.zip', 1, '', '', 'so', 'o')",
        "INSERT INTO MEDIA (BOOK_ID, STATUS, MEDIA_TYPE) VALUES \
         ('b1', 'UNKNOWN', 'application/zip'), ('b2', 'OUTDATED', 'application/zip'), \
         ('b3', 'UNKNOWN', NULL), ('b4', 'READY', 'application/zip'), \
         ('bo', 'UNKNOWN', 'application/zip')",
    ] {
        sqlx::query(statement)
            .execute(tx.connection())
            .await
            .expect("Fixture");
    }
}

#[tokio::test]
async fn finds_the_books_scan_follow_ups_queue_tasks_for() {
    let directory = komga_test_support::tempdir().expect("Database directory");
    let db = Database::connect(&directory.path().join("database.sqlite"))
        .await
        .expect("Open database");
    db.migrate().await.expect("Migrate database");
    let mut tx = db.begin().await.expect("Begin");
    insert_books(&mut tx).await;
    let library = LibraryId::from("l".to_owned());

    let to_analyze = BookDao::find_all_by_library_and_media_statuses(
        &mut tx,
        &library,
        &["UNKNOWN", "OUTDATED"],
    )
    .await
    .expect("By status");
    assert_eq!(ids(&to_analyze), ["b3", "b2", "b1"], "series, then number");
    let mismatched = BookDao::find_all_by_library_and_mismatched_extension(
        &mut tx,
        &library,
        "application/zip",
        "cbz",
    )
    .await
    .expect("Mismatched");
    assert_eq!(
        ids(&mismatched),
        ["b2"],
        "`b1.CBZ` matches case-insensitively"
    );
    for (hash, expected) in [(BookHash::File, "b1"), (BookHash::Koreader, "b2")] {
        let books = BookDao::find_all_by_library_with_empty_hash(&mut tx, &library, hash)
            .await
            .expect("Empty hash");
        assert_eq!(ids(&books), [expected], "{hash:?}");
    }
}
