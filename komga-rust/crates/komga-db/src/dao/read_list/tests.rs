//! `ReadListDao` against a migrated database.

use super::ReadListDao;
use crate::Database;

#[tokio::test]
async fn deletes_only_read_lists_without_books_and_their_thumbnails() {
    let directory = komga_test_support::tempdir().expect("Database directory");
    let db = Database::connect(&directory.path().join("database.sqlite"))
        .await
        .expect("Open database");
    db.migrate().await.expect("Migrate database");
    let mut tx = db.begin_immediate().await.expect("Begin");
    for statement in [
        "INSERT INTO LIBRARY (ID, NAME, ROOT) VALUES ('l', 'L', 'file:/l')",
        "INSERT INTO SERIES (ID, FILE_LAST_MODIFIED, NAME, URL, LIBRARY_ID) \
         VALUES ('s', CURRENT_TIMESTAMP, 'S', 'file:/l/s', 'l')",
        "INSERT INTO BOOK (ID, FILE_LAST_MODIFIED, NAME, URL, SERIES_ID, LIBRARY_ID) \
         VALUES ('b', CURRENT_TIMESTAMP, 'B', 'file:/l/s/b.cbz', 's', 'l')",
        "INSERT INTO READLIST (ID, NAME, BOOK_COUNT) VALUES ('kept', 'K', 1), ('empty', 'E', 0)",
        "INSERT INTO READLIST_BOOK (READLIST_ID, BOOK_ID, NUMBER) VALUES ('kept', 'b', 0)",
        "INSERT INTO THUMBNAIL_READLIST (ID, THUMBNAIL, TYPE, READLIST_ID) \
         VALUES ('t1', x'00', 'USER_UPLOADED', 'kept'), ('t2', x'00', 'USER_UPLOADED', 'empty')",
    ] {
        sqlx::query(statement)
            .execute(tx.connection())
            .await
            .expect("Fixture");
    }
    assert_eq!(
        ReadListDao::delete_empty(&mut tx).await.expect("Delete"),
        [komga_types::ReadListId::from("empty".to_owned())]
    );
    let remaining: Vec<(String, String)> = sqlx::query_as(
        "SELECT r.ID, t.ID FROM READLIST r JOIN THUMBNAIL_READLIST t ON t.READLIST_ID = r.ID",
    )
    .fetch_all(tx.connection())
    .await
    .expect("Remaining");
    assert_eq!(remaining, [("kept".to_owned(), "t1".to_owned())]);
    let thumbnails: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM THUMBNAIL_READLIST")
        .fetch_one(tx.connection())
        .await
        .expect("Thumbnails");
    assert_eq!(thumbnails, 1, "the empty read list's thumbnail is deleted");
}
