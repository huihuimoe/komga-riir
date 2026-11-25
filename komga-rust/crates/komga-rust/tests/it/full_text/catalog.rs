//! Book and series lists and groups with `fullTextSearch`, and their deprecated `GET` forms with
//! `search`.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{LIBRARY_A_USER, get, post, values};
use crate::read_lists::{Fixture, create_user, fixture, send};
use crate::support::search::wait_for_index;

async fn edit(f: &Fixture, path: &str, body: Value) {
    let response = send(&f.server, None, Method::PATCH, path, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT, "{path}");
}

/// Titles the books `Batman` (`a1`), `Batman Returns Again` (`a2`) and `Batman Beyond` (`b1`), and
/// the series `Gotham` (`SA`) and `Metropolis` (`SB`); creates a user of library `A`.
async fn titled(f: &Fixture) {
    for (book, title) in [
        (&f.a1, "Batman"),
        (&f.a2, "Batman Returns Again"),
        (&f.b1, "Batman Beyond"),
    ] {
        edit(
            f,
            &format!("/api/v1/books/{book}/metadata"),
            json!({"title": title}),
        )
        .await;
    }
    for (series, title) in [(&f.series_a, "Gotham"), (&f.series_b, "Metropolis")] {
        let body = json!({"title": title, "titleSort": title});
        edit(f, &format!("/api/v1/series/{series}/metadata"), body).await;
    }
    let access = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, LIBRARY_A_USER, access).await;
    wait_for_index(&f.pool).await;
}

fn titles(page: &Value) -> Vec<String> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|book| {
            book["metadata"]["title"]
                .as_str()
                .expect("Title")
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn books_are_searched_within_conditions_and_visible_content() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    titled(&f).await;
    let batman = json!({"fullTextSearch": "batman"});
    const LIST: &str = "/api/v1/books/list";

    let relevant = post(&f, None, LIST, &batman).await;
    assert_eq!(
        titles(&relevant),
        ["Batman", "Batman Beyond", "Batman Returns Again"]
    );
    assert_eq!(relevant["sort"]["sorted"], true);
    let by_name = post(&f, None, &format!("{LIST}?sort=name,desc"), &batman).await;
    assert_eq!(values(&by_name, "name"), ["b1", "a2", "a1"]);
    let in_b = json!({
        "fullTextSearch": "batman",
        "condition": {"libraryId": {"operator": "isNot", "value": &f.library_a}},
    });
    assert_eq!(
        titles(&post(&f, None, LIST, &in_b).await),
        ["Batman Beyond"]
    );
    let mine = post(&f, Some(LIBRARY_A_USER), &format!("{LIST}?size=1"), &batman).await;
    assert_eq!(
        (titles(&mine), &mine["totalElements"]),
        (vec!["Batman".to_owned()], &json!(2))
    );
    let blank = post(&f, None, LIST, &json!({"fullTextSearch": " "})).await;
    assert_eq!(
        (&blank["totalElements"], &blank["sort"]["sorted"]),
        (&json!(3), &json!(false))
    );

    let path = format!("/api/v1/books?search=batman&library_id={}", f.library_a);
    let found = get(&f, None, &path).await;
    assert_eq!(titles(&found), ["Batman", "Batman Returns Again"]);
    let unsearched = get(&f, None, "/api/v1/books?sort=relevance").await;
    assert_eq!(unsearched["sort"]["sorted"], false);
    f.server.stop().await;
}

#[tokio::test]
async fn series_are_searched_in_lists_and_groups() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    titled(&f).await;
    let metropolis = json!({"fullTextSearch": "metropolis"});
    let mine = Some(LIBRARY_A_USER);
    const LIST: &str = "/api/v1/series/list";
    const GROUPS: &str = "/api/v1/series/list/alphabetical-groups";

    let found = post(&f, None, LIST, &metropolis).await;
    assert_eq!(values(&found, "name"), ["SB"]);
    assert_eq!(found["totalElements"], 1);
    assert_eq!(post(&f, mine, LIST, &metropolis).await["totalElements"], 0);
    let grouped = post(&f, None, GROUPS, &metropolis).await;
    assert_eq!(grouped, json!([{"group": "m", "count": 1}]));
    assert_eq!(post(&f, mine, GROUPS, &metropolis).await, json!([]));

    let both = get(
        &f,
        None,
        "/api/v1/series?search=gotham%20OR%20metropolis&sort=name",
    )
    .await;
    assert_eq!(values(&both, "name"), ["SA", "SB"]);
    let path = format!(
        "/api/v1/series?search=metropolis&library_id={}",
        f.library_a
    );
    assert_eq!(get(&f, None, &path).await["totalElements"], 0);
    let grouped = get(&f, None, "/api/v1/series/alphabetical-groups?search=gotham").await;
    assert_eq!(grouped, json!([{"group": "g", "count": 1}]));
    f.server.stop().await;
}
