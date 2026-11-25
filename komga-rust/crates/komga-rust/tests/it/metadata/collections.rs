//! `RefreshSeriesMetadata` adding series to the collections of their books' `ComicInfo.xml`.

use komga_types::SearchEntity;
use sqlx::SqlitePool;

use super::{comic, library, rescan, rescan_deep, write};
use crate::support::search::{hits, wait_for_index};

/// `(COLLECTION.NAME, SERIES_COUNT, SERIES.NAME, NUMBER)` by collection name and number.
type Member = (String, i64, String, i64);

async fn members(pool: &SqlitePool) -> Vec<Member> {
    sqlx::query_as(
        "SELECT c.NAME, c.SERIES_COUNT, s.NAME, cs.NUMBER FROM COLLECTION c \
         JOIN COLLECTION_SERIES cs ON cs.COLLECTION_ID = c.ID JOIN SERIES s ON s.ID = cs.SERIES_ID \
         ORDER BY c.NAME, cs.NUMBER",
    )
    .fetch_all(pool)
    .await
    .expect("Collection members")
}

fn member(collection: &str, count: i64, series: &str, number: i64) -> Member {
    (collection.to_owned(), count, series.to_owned(), number)
}

#[tokio::test]
async fn series_groups_add_each_series_once_after_the_last_one() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let first = "<ComicInfo><SeriesGroup>Group, Other</SeriesGroup></ComicInfo>";
    write(directory.path(), "A/a.cbz", &comic(Some(first)));
    let (server, library, pool) = library(directory.path()).await;
    assert_eq!(
        members(&pool).await,
        [member("Group", 1, "A", 0), member("Other", 1, "A", 0)]
    );

    // `group` names the same collection, which gains `B` after `A`.
    let second = "<ComicInfo><SeriesGroup>group</SeriesGroup></ComicInfo>";
    write(directory.path(), "B/b.cbz", &comic(Some(second)));
    rescan(&server, directory.path(), &library).await;
    let expected = [
        member("Group", 2, "A", 0),
        member("Group", 2, "B", 1),
        member("Other", 1, "A", 0),
    ];
    assert_eq!(members(&pool).await, expected);

    rescan_deep(&server, directory.path(), &library).await;
    assert_eq!(
        members(&pool).await,
        expected,
        "refreshing again adds no series twice"
    );
    wait_for_index(&pool).await;
    server.stop().await;
    let found = |query| hits(directory.path(), SearchEntity::Collection, query).len();
    assert_eq!([found("*:*"), found("group"), found("other")], [2, 1, 1]);
}
