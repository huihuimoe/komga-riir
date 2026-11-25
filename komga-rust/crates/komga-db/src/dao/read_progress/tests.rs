use komga_types::BookId;

use super::ReadProgressDao;
use crate::Database;

type Progress = (String, String, i32, bool);
type SeriesProgress = (String, String, i32, i32, Option<String>);

#[tokio::test]
async fn resets_the_pages_of_one_book_and_reaggregates_its_series() {
    let directory = komga_test_support::tempdir().expect("Database directory");
    let db = Database::connect(&directory.path().join("database.sqlite"))
        .await
        .expect("Open database");
    db.migrate().await.expect("Migrate database");
    let mut tx = db.begin().await.expect("Begin");
    // `s1` holds `b1` and `b2`; `s2` holds `b3`. The series rows start stale.
    for statement in [
        "INSERT INTO LIBRARY (ID, NAME, ROOT) VALUES ('l', 'L', 'file:/l/')",
        "INSERT INTO USER (ID, EMAIL, PASSWORD) VALUES ('u1', 'u1@x', 'p'), ('u2', 'u2@x', 'p')",
        "INSERT INTO SERIES (ID, FILE_LAST_MODIFIED, NAME, URL, LIBRARY_ID) VALUES \
         ('s1', CURRENT_TIMESTAMP, 'S1', 'file:/l/s1/', 'l'), \
         ('s2', CURRENT_TIMESTAMP, 'S2', 'file:/l/s2/', 'l')",
        "INSERT INTO BOOK (ID, FILE_LAST_MODIFIED, NAME, URL, NUMBER, SERIES_ID, LIBRARY_ID) \
         VALUES ('b1', CURRENT_TIMESTAMP, 'b1', 'file:/l/s1/b1.cbz', 1, 's1', 'l'), \
         ('b2', CURRENT_TIMESTAMP, 'b2', 'file:/l/s1/b2.cbz', 2, 's1', 'l'), \
         ('b3', CURRENT_TIMESTAMP, 'b3', 'file:/l/s2/b3.cbz', 1, 's2', 'l')",
        "INSERT INTO READ_PROGRESS (BOOK_ID, USER_ID, PAGE, COMPLETED, READ_DATE) VALUES \
         ('b1', 'u1', 10, 1, '2024-01-03 00:00:00'), ('b2', 'u1', 3, 0, '2024-01-05 00:00:00'), \
         ('b1', 'u2', 5, 0, '2024-01-02 00:00:00'), ('b3', 'u1', 7, 0, '2024-01-01 00:00:00')",
        "INSERT INTO READ_PROGRESS_SERIES (SERIES_ID, USER_ID, READ_COUNT, IN_PROGRESS_COUNT) \
         VALUES ('s1', 'u1', 9, 9), ('s1', 'u2', 9, 9), ('s2', 'u1', 9, 9)",
    ] {
        sqlx::query(statement)
            .execute(tx.connection())
            .await
            .expect("Fixture");
    }

    ReadProgressDao::reset_pages(&mut tx, &BookId::from("b1".to_owned()), 4)
        .await
        .expect("Reset pages");

    let progress: Vec<Progress> = sqlx::query_as(
        "SELECT BOOK_ID, USER_ID, PAGE, COMPLETED FROM READ_PROGRESS ORDER BY BOOK_ID, USER_ID",
    )
    .fetch_all(tx.connection())
    .await
    .expect("Progress");
    let row = |book: &str, user: &str, page, completed| {
        (book.to_owned(), user.to_owned(), page, completed)
    };
    assert_eq!(
        progress,
        [
            row("b1", "u1", 4, true),
            row("b1", "u2", 1, false),
            row("b2", "u1", 3, false),
            row("b3", "u1", 7, false),
        ]
    );
    let series: Vec<SeriesProgress> = sqlx::query_as(
        "SELECT SERIES_ID, USER_ID, READ_COUNT, IN_PROGRESS_COUNT, MOST_RECENT_READ_DATE \
         FROM READ_PROGRESS_SERIES ORDER BY SERIES_ID, USER_ID",
    )
    .fetch_all(tx.connection())
    .await
    .expect("Series progress");
    let date = |value: &str| Some(value.to_owned());
    assert_eq!(
        series,
        [
            ("s1".into(), "u1".into(), 1, 1, date("2024-01-05 00:00:00")),
            ("s1".into(), "u2".into(), 0, 1, date("2024-01-02 00:00:00")),
            ("s2".into(), "u1".into(), 9, 9, None),
        ]
    );
}
