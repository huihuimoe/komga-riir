use komga_types::{BookId, SeriesId};

use super::RestoreDao;
use crate::{Database, DbTransaction};

/// A library with the soft-deleted series `old` and the new series `new`.
async fn database(
    directory: &komga_test_support::TestDir,
    statements: &[&'static str],
) -> (Database, DbTransaction) {
    let db = Database::connect(&directory.path().join("database.sqlite"))
        .await
        .expect("Open database");
    db.migrate().await.expect("Migrate database");
    let mut tx = db.begin_immediate().await.expect("Begin");
    let base = [
        "INSERT INTO LIBRARY (ID, NAME, ROOT) VALUES ('l', 'L', 'file:/l/')",
        "INSERT INTO USER (ID, EMAIL, PASSWORD) VALUES ('u', 'u@x', 'p')",
        "INSERT INTO SERIES (ID, FILE_LAST_MODIFIED, NAME, URL, LIBRARY_ID, DELETED_DATE) VALUES \
         ('old', CURRENT_TIMESTAMP, 'Old', 'file:/l/old/', 'l', CURRENT_TIMESTAMP), \
         ('new', CURRENT_TIMESTAMP, 'New', 'file:/l/new/', 'l', NULL)",
    ];
    for &statement in base.iter().chain(statements) {
        sqlx::query(statement)
            .execute(tx.connection())
            .await
            .expect("Fixture");
    }
    (db, tx)
}

async fn rows<T>(tx: &mut DbTransaction, query: &'static str) -> Vec<T>
where
    T: Send + Unpin + for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow>,
{
    sqlx::query_as(query)
        .fetch_all(tx.connection())
        .await
        .expect("Query")
}

/// `o1` and `o2` were deleted with `old`; `n1` and `n2` are new. Only `o2` locked its title.
const BOOKS: &[&str] = &[
    "INSERT INTO BOOK (ID, FILE_LAST_MODIFIED, NAME, URL, SERIES_ID, LIBRARY_ID, \
     FILE_HASH_KOREADER, DELETED_DATE) VALUES \
     ('o1', CURRENT_TIMESTAMP, 'o1', 'file:/l/old/1', 'old', 'l', 'k1', CURRENT_TIMESTAMP), \
     ('o2', CURRENT_TIMESTAMP, 'o2', 'file:/l/old/2', 'old', 'l', 'k2', CURRENT_TIMESTAMP), \
     ('n1', CURRENT_TIMESTAMP, 'n1', 'file:/l/new/1', 'new', 'l', '', NULL), \
     ('n2', CURRENT_TIMESTAMP, 'n2', 'file:/l/new/2', 'new', 'l', '', NULL)",
    "INSERT INTO MEDIA (BOOK_ID, STATUS, PAGE_COUNT) VALUES \
     ('o1', 'READY', 1), ('o2', 'READY', 1), ('n1', 'UNKNOWN', 0), ('n2', 'UNKNOWN', 0)",
    "INSERT INTO MEDIA_PAGE (BOOK_ID, FILE_NAME, MEDIA_TYPE, NUMBER) VALUES \
     ('o1', 'p.png', 'image/png', 0)",
    "INSERT INTO BOOK_METADATA (BOOK_ID, TITLE, TITLE_LOCK, NUMBER, NUMBER_SORT) VALUES \
     ('o1', 'Old 1', 0, '1', 1), ('o2', 'Old 2', 1, '2', 2), \
     ('n1', 'New 1', 0, '1', 1), ('n2', 'New 2', 0, '2', 2)",
    "INSERT INTO BOOK_METADATA_AUTHOR (BOOK_ID, NAME, ROLE) VALUES ('o1', 'A', 'writer')",
    "INSERT INTO THUMBNAIL_BOOK (ID, BOOK_ID, TYPE) VALUES \
     ('generated', 'o1', 'GENERATED'), ('uploaded', 'o1', 'USER_UPLOADED'), \
     ('sidecar', 'o1', 'SIDECAR')",
    "INSERT INTO READ_PROGRESS (BOOK_ID, USER_ID, PAGE, COMPLETED, READ_DATE) VALUES \
     ('o1', 'u', 1, 1, '2024-01-01 00:00:00')",
    "INSERT INTO READ_PROGRESS_SERIES (SERIES_ID, USER_ID, READ_COUNT, IN_PROGRESS_COUNT) \
     VALUES ('old', 'u', 1, 0)",
    "INSERT INTO READLIST (ID, NAME, BOOK_COUNT, LAST_MODIFIED_DATE) VALUES \
     ('r', 'R', 1, '2000-01-01 00:00:00')",
    "INSERT INTO READLIST_BOOK (READLIST_ID, BOOK_ID, NUMBER) VALUES ('r', 'o1', 0)",
];

#[tokio::test]
async fn a_restored_book_hands_over_its_state_and_is_deleted() {
    let directory = komga_test_support::tempdir().expect("Database directory");
    let (_db, mut tx) = database(&directory, BOOKS).await;

    let book = |id: &str| BookId::from(id);
    let kept_title = RestoreDao::restore_book(&mut tx, &book("o1"), &book("n1"))
        .await
        .expect("Restore o1");
    assert!(kept_title);
    let kept_title = RestoreDao::restore_book(&mut tx, &book("o2"), &book("n2"))
        .await
        .expect("Restore o2");
    assert!(!kept_title);

    let books: Vec<(String, String)> = rows(
        &mut tx,
        "SELECT ID, FILE_HASH_KOREADER FROM BOOK ORDER BY ID",
    )
    .await;
    assert_eq!(
        books,
        [("n1".into(), "k1".into()), ("n2".into(), "k2".into())]
    );
    let media: Vec<(String, String, i64)> = rows(
        &mut tx,
        "SELECT m.BOOK_ID, m.STATUS, (SELECT COUNT(*) FROM MEDIA_PAGE p WHERE p.BOOK_ID = m.BOOK_ID) \
         FROM MEDIA m ORDER BY m.BOOK_ID",
    )
    .await;
    assert_eq!(
        media,
        [
            ("n1".into(), "READY".into(), 1),
            ("n2".into(), "READY".into(), 0)
        ]
    );
    let titles: Vec<(String, String, i64)> = rows(
        &mut tx,
        "SELECT d.BOOK_ID, d.TITLE, (SELECT COUNT(*) FROM BOOK_METADATA_AUTHOR a \
         WHERE a.BOOK_ID = d.BOOK_ID) FROM BOOK_METADATA d ORDER BY d.BOOK_ID",
    )
    .await;
    assert_eq!(
        titles,
        [
            ("n1".into(), "New 1".into(), 1),
            ("n2".into(), "Old 2".into(), 0)
        ]
    );
    let thumbnails: Vec<(String, String)> = rows(
        &mut tx,
        "SELECT ID, BOOK_ID FROM THUMBNAIL_BOOK ORDER BY ID",
    )
    .await;
    assert_eq!(
        thumbnails,
        [
            ("generated".into(), "n1".into()),
            ("uploaded".into(), "n1".into())
        ]
    );
    let progress: Vec<(String, String)> =
        rows(&mut tx, "SELECT BOOK_ID, USER_ID FROM READ_PROGRESS").await;
    assert_eq!(progress, [("n1".into(), "u".into())]);
    let series_progress: Vec<(String, i32)> = rows(
        &mut tx,
        "SELECT SERIES_ID, READ_COUNT FROM READ_PROGRESS_SERIES",
    )
    .await;
    assert_eq!(series_progress, [("new".into(), 1)]);
    let read_lists: Vec<(String, bool)> = rows(
        &mut tx,
        "SELECT b.BOOK_ID, r.LAST_MODIFIED_DATE > '2000-01-01 00:00:00' \
         FROM READLIST_BOOK b JOIN READLIST r ON r.ID = b.READLIST_ID",
    )
    .await;
    assert_eq!(read_lists, [("n1".into(), true)]);
}

#[tokio::test]
async fn a_restored_series_hands_over_its_state_and_is_deleted_with_its_books() {
    let directory = komga_test_support::tempdir().expect("Database directory");
    // `old` locked its title only and keeps an unmatched book.
    let (_db, mut tx) = database(&directory, &[
        "INSERT INTO BOOK (ID, FILE_LAST_MODIFIED, NAME, URL, SERIES_ID, LIBRARY_ID, \
         DELETED_DATE) VALUES \
         ('left', CURRENT_TIMESTAMP, 'left', 'file:/l/old/1', 'old', 'l', CURRENT_TIMESTAMP)",
        "INSERT INTO SERIES_METADATA (SERIES_ID, STATUS, TITLE, TITLE_LOCK, TITLE_SORT, \
         TITLE_SORT_LOCK) VALUES ('old', 'ENDED', 'Old', 1, 'old', 0), \
         ('new', 'ONGOING', 'New', 0, 'new', 0)",
        "INSERT INTO SERIES_METADATA_GENRE (SERIES_ID, GENRE) VALUES ('old', 'g')",
        "INSERT INTO THUMBNAIL_SERIES (ID, SERIES_ID, TYPE) VALUES \
         ('uploaded', 'old', 'USER_UPLOADED'), ('sidecar', 'old', 'SIDECAR')",
        "INSERT INTO COLLECTION (ID, NAME, SERIES_COUNT, LAST_MODIFIED_DATE) VALUES \
         ('c', 'C', 1, '2000-01-01 00:00:00')",
        "INSERT INTO COLLECTION_SERIES (COLLECTION_ID, SERIES_ID, NUMBER) VALUES ('c', 'old', 0)",
    ])
    .await;

    let series = |id: &str| SeriesId::from(id);
    RestoreDao::restore_series(&mut tx, &series("old"), &series("new"))
        .await
        .expect("Restore series");

    let remaining: Vec<(String,)> = rows(
        &mut tx,
        "SELECT ID FROM SERIES UNION ALL SELECT ID FROM BOOK",
    )
    .await;
    assert_eq!(remaining, [("new".into(),)]);
    let metadata: Vec<(String, String, String, String)> = rows(
        &mut tx,
        "SELECT SERIES_ID, STATUS, TITLE, TITLE_SORT FROM SERIES_METADATA",
    )
    .await;
    assert_eq!(
        metadata,
        [("new".into(), "ENDED".into(), "Old".into(), "new".into())]
    );
    let genres: Vec<(String, String)> = rows(
        &mut tx,
        "SELECT SERIES_ID, GENRE FROM SERIES_METADATA_GENRE",
    )
    .await;
    assert_eq!(genres, [("new".into(), "g".into())]);
    let thumbnails: Vec<(String, String)> =
        rows(&mut tx, "SELECT ID, SERIES_ID FROM THUMBNAIL_SERIES").await;
    assert_eq!(thumbnails, [("uploaded".into(), "new".into())]);
    let collections: Vec<(String, bool)> = rows(
        &mut tx,
        "SELECT s.SERIES_ID, c.LAST_MODIFIED_DATE > '2000-01-01 00:00:00' \
         FROM COLLECTION_SERIES s JOIN COLLECTION c ON c.ID = s.COLLECTION_ID",
    )
    .await;
    assert_eq!(collections, [("new".into(), true)]);
}
