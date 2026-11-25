//! `GET /api/v1/readlists` and `GET /api/v1/collections` with `search`.

use std::collections::BTreeSet;

use komga_types::SearchEntity;
use serde_json::{Value, json};

use super::{LIBRARY_A_USER, get, post, values};
use crate::read_lists::{Fixture, create_list, create_user, fixture};
use crate::support::search::{hits, wait_for_index};

const READ_LISTS: &str = "/api/v1/readlists";
const COLLECTIONS: &str = "/api/v1/collections";
/// More than the 1000 hits Kotlin keeps, and more than one batch of candidates.
const LISTS: usize = 1100;

async fn library_a_user(f: &Fixture) {
    let access = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, LIBRARY_A_USER, access).await;
}

/// The number in the name `Batman <number>`.
fn number(name: &str) -> usize {
    name.strip_prefix("Batman ")
        .and_then(|number| number.parse().ok())
        .expect("A numbered list")
}

fn names(page: &Value) -> Vec<String> {
    values(page, "name")
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// Seeds `count` alternating `Batman <n>` read lists in one transaction so the test avoids 1100
/// sequential HTTP POSTs; the caller's next HTTP mutation wakes the search index updater.
async fn seed_read_lists(f: &Fixture, count: usize) {
    let mut tx = f.pool.begin().await.expect("Begin bulk readlist seed");
    for n in 0..count {
        let id = format!("RL{n:04}");
        let name = format!("Batman {n}");
        let book = if n.is_multiple_of(2) { &f.a1 } else { &f.b1 };
        sqlx::query("INSERT INTO READLIST (ID, NAME, BOOK_COUNT) VALUES (?, ?, 1)")
            .bind(&id)
            .bind(&name)
            .execute(&mut *tx)
            .await
            .expect("Insert read list");
        sqlx::query("INSERT INTO READLIST_BOOK (READLIST_ID, BOOK_ID, NUMBER) VALUES (?, ?, 0)")
            .bind(&id)
            .bind(book)
            .execute(&mut *tx)
            .await
            .expect("Insert read list book");
        sqlx::query(
            "INSERT OR REPLACE INTO ERA_SEARCH_INDEX_PENDING (ENTITY, ENTITY_ID) VALUES (?, ?)",
        )
        .bind(SearchEntity::ReadList.type_name())
        .bind(&id)
        .execute(&mut *tx)
        .await
        .expect("Mark read list pending");
    }
    tx.commit().await.expect("Commit bulk readlist seed");
}

#[tokio::test]
async fn every_hit_of_a_large_search_is_filtered_counted_and_paged() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    seed_read_lists(&f, LISTS).await;
    create_list(&f, "Superman", &[&f.a2]).await;
    library_a_user(&f).await;
    wait_for_index(&f.pool).await;
    let mine = Some(LIBRARY_A_USER);

    let mut seen = BTreeSet::new();
    for page in 0..3 {
        let path = format!("{READ_LISTS}?search=batman&size=500&page={page}");
        let paged = get(&f, None, &path).await;
        assert_eq!(paged["totalElements"], LISTS);
        assert_eq!(paged["sort"]["sorted"], true, "by relevance");
        seen.extend(values(&paged, "name").into_iter().map(number));
    }
    assert_eq!(seen, (0..LISTS).collect());

    let visible = get(
        &f,
        mine,
        &format!("{READ_LISTS}?search=batman&unpaged=true"),
    )
    .await;
    let visible = names(&visible);
    assert_eq!(visible.len(), LISTS / 2);
    assert!(
        visible.iter().all(|name| number(name).is_multiple_of(2)),
        "{visible:?}"
    );
    let last = get(
        &f,
        mine,
        &format!("{READ_LISTS}?search=batman&size=20&page=27"),
    )
    .await;
    assert_eq!(
        (names(&last).len(), &last["totalElements"]),
        (10, &json!(LISTS / 2))
    );
    let path = format!("{READ_LISTS}?search=batman&library_id={}", f.library_a);
    assert_eq!(get(&f, None, &path).await["totalElements"], LISTS / 2);

    let by_name = get(
        &f,
        None,
        &format!("{READ_LISTS}?search=batman&sort=name&size=3"),
    )
    .await;
    assert_eq!(names(&by_name), ["Batman 0", "Batman 1", "Batman 2"]);
    assert_eq!(by_name["totalElements"], LISTS);
    f.server.stop().await;
}

#[tokio::test]
async fn searches_order_by_relevance_unless_sorted() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    for (name, series) in [
        ("Batman Returns Again", &f.series_a),
        ("Superman", &f.series_b),
        ("Batman", &f.series_a),
        ("Batman Beyond", &f.series_b),
    ] {
        let body = json!({"name": name, "seriesIds": [series]});
        post(&f, None, COLLECTIONS, &body).await;
    }
    library_a_user(&f).await;
    wait_for_index(&f.pool).await;
    let list = |query: &str| format!("{COLLECTIONS}?{query}");

    let relevant = get(&f, None, &list("search=batman")).await;
    assert_eq!(
        names(&relevant),
        ["Batman", "Batman Beyond", "Batman Returns Again"]
    );
    assert_eq!(relevant["sort"]["sorted"], true);
    let reversed = get(&f, None, &list("search=batman&sort=relevance,desc")).await;
    assert_eq!(
        names(&reversed),
        ["Batman Returns Again", "Batman Beyond", "Batman"]
    );
    let second = get(&f, None, &list("search=batman&size=1&page=1")).await;
    assert_eq!(
        (names(&second), &second["totalElements"]),
        (vec!["Batman Beyond".to_owned()], &json!(3))
    );
    let mine = get(&f, Some(LIBRARY_A_USER), &list("search=batman")).await;
    assert_eq!(names(&mine), ["Batman", "Batman Returns Again"]);
    assert_eq!(mine["totalElements"], 2);

    let blank = get(&f, None, &list("search=%20")).await;
    assert_eq!(
        names(&blank),
        [
            "Batman",
            "Batman Beyond",
            "Batman Returns Again",
            "Superman"
        ],
        "by name"
    );
    let unsearched = get(&f, None, &list("sort=relevance")).await;
    assert_eq!(
        (&unsearched["totalElements"], &unsearched["sort"]["sorted"]),
        (&json!(4), &json!(false))
    );
    for query in ["search=joker", "search=batman%20AND%20("] {
        let none = get(&f, None, &list(query)).await;
        assert_eq!(
            (names(&none).len(), &none["totalElements"]),
            (0, &json!(0)),
            "{query}"
        );
    }

    create_list(&f, "Batman Beyond", &[&f.b1]).await;
    create_list(&f, "Batman", &[&f.a1]).await;
    wait_for_index(&f.pool).await;
    let lists = get(&f, None, &format!("{READ_LISTS}?search=batman")).await;
    assert_eq!(names(&lists), ["Batman", "Batman Beyond"]);
    f.server.stop().await;
}

#[tokio::test]
async fn a_stale_index_does_not_widen_what_is_visible() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    create_list(&f, "Batman kept", &[&f.a1]).await;
    let moved = create_list(&f, "Batman moved", &[&f.a1]).await;
    let deleted = create_list(&f, "Batman deleted", &[&f.a1]).await;
    library_a_user(&f).await;
    wait_for_index(&f.pool).await;
    let path = format!("{READ_LISTS}?search=batman");
    assert_eq!(
        get(&f, Some(LIBRARY_A_USER), &path).await["totalElements"],
        3
    );

    // Changed behind the index, which keeps all three lists.
    let changes = [
        sqlx::query("UPDATE READLIST_BOOK SET BOOK_ID = ? WHERE READLIST_ID = ?")
            .bind(&f.b1)
            .bind(&moved),
        sqlx::query("DELETE FROM READLIST_BOOK WHERE READLIST_ID = ?").bind(&deleted),
        sqlx::query("DELETE FROM READLIST WHERE ID = ?").bind(&deleted),
    ];
    for change in changes {
        change
            .execute(&f.pool)
            .await
            .expect("Change behind the index");
    }

    let mine = get(&f, Some(LIBRARY_A_USER), &path).await;
    assert_eq!(names(&mine), ["Batman kept"]);
    assert_eq!(mine["totalElements"], 1);
    let all = get(&f, None, &format!("{path}&sort=name,asc")).await;
    assert_eq!(names(&all), ["Batman kept", "Batman moved"]);
    assert_eq!(all["totalElements"], 2);
    f.server.stop().await;
    let indexed = hits(directory.path(), SearchEntity::ReadList, "batman");
    assert_eq!(indexed.len(), 3, "the index was not updated");
}
