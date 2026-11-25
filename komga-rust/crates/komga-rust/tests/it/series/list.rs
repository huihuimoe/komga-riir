//! `POST /api/v1/series/list` (`SeriesController.getSeries`) with search conditions.

use std::path::Path;

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::details::{comic, mark};
use crate::books::library_with_books;
use crate::read_lists::{create_user, rate, send};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

const LIST: &str = "/api/v1/series/list";
const PLAIN: &str = "<ComicInfo></ComicInfo>";

/// Series `Alpha` (a1, a2), `Beta` (b1, by Ann with Bob, tagged and dated) and `Gamma`
/// (g1–g3) in one library.
pub(crate) async fn three_series(directory: &Path) -> (TestServer, Value, SqlitePool) {
    let beta = "<ComicInfo><Writer>Ann</Writer><Characters>Bob</Characters>\
                <Tags>Book Tag</Tags><Year>2020</Year><Month>1</Month><Day>2</Day></ComicInfo>";
    let books = [
        ("Alpha/a1.cbz", comic(PLAIN)),
        ("Alpha/a2.cbz", comic(PLAIN)),
        ("Beta/b1.cbz", comic(beta)),
        ("Gamma/g1.cbz", comic(PLAIN)),
        ("Gamma/g2.cbz", comic(PLAIN)),
        ("Gamma/g3.cbz", comic(PLAIN)),
    ];
    library_with_books(directory, &books).await
}

/// The page `email` (else the admin) gets for `query` and `body`.
async fn search(server: &TestServer, email: Option<&str>, query: &str, body: &Value) -> Value {
    let path = format!("{LIST}{query}");
    let response = send(server, email, Method::POST, &path, Some(body)).await;
    assert_eq!(response.status(), StatusCode::OK, "{body}");
    openapi::checked(Method::POST, response).await
}

pub(crate) fn names(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|series| series["name"].as_str().expect("Series name"))
        .collect()
}

/// Names of the series matching `condition`, by name.
async fn matching(server: &TestServer, condition: &Value) -> Vec<String> {
    let body = json!({"condition": condition});
    let page = search(server, None, "?unpaged=true&sort=name", &body).await;
    names(&page).into_iter().map(str::to_owned).collect()
}

async fn assert_matches(server: &TestServer, cases: Vec<(Value, Vec<&str>)>) {
    for (condition, expected) in cases {
        assert_eq!(matching(server, &condition).await, expected, "{condition}");
    }
}

fn is(value: &str) -> Value {
    json!({"operator": "is", "value": value})
}

fn is_not(value: &str) -> Value {
    json!({"operator": "isNot", "value": value})
}

fn operator(name: &str) -> Value {
    json!({"operator": name})
}

pub(crate) async fn edit_series(server: &TestServer, series_id: &str, body: &Value) {
    let path = format!("/api/v1/series/{series_id}/metadata");
    let response = send(server, None, Method::PATCH, &path, Some(body)).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn pages_count_and_sort_every_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, _) = three_series(directory.path()).await;
    let all = json!({});

    let first = search(&server, None, "?size=2&sort=name", &all).await;
    assert_eq!(names(&first), ["Alpha", "Beta"]);
    assert_eq!(first["totalElements"], 3);
    assert_eq!(first["totalPages"], 2);
    assert_eq!(first["sort"]["sorted"], true);
    assert_eq!(first["content"][0]["booksCount"], 2);
    let last = search(&server, None, "?size=2&page=1&sort=name", &all).await;
    assert_eq!(names(&last), ["Gamma"]);
    let past = search(&server, None, "?size=2&page=4&sort=name", &all).await;
    assert_eq!(names(&past), Vec::<&str>::new());
    assert_eq!(past["totalElements"], 3);

    let by_count = search(&server, None, "?sort=booksCount,desc", &all).await;
    assert_eq!(names(&by_count), ["Gamma", "Alpha", "Beta"]);
    let by_date = search(&server, None, "?sort=booksMetadata.releaseDate,desc", &all).await;
    assert_eq!(names(&by_date)[0], "Beta");
    let random = search(&server, None, "?sort=random", &all).await;
    assert_eq!(random["totalElements"], 3);
    let unknown = search(&server, None, "?sort=unknown", &all).await;
    assert_eq!(unknown["sort"]["sorted"], false);
    let unpaged = search(
        &server,
        None,
        "?unpaged=true&sort=metadata.titleSort,desc",
        &all,
    )
    .await;
    assert_eq!(names(&unpaged), ["Gamma", "Beta", "Alpha"]);
    assert_eq!(unpaged["size"], 20);
    let empty_any = json!({"condition": {"anyOf": []}});
    assert_eq!(
        search(&server, None, "", &empty_any).await["totalElements"],
        3
    );

    for body in [
        json!({"condition": {"title": {"operator": "nope", "value": "a"}}}),
        json!({"condition": {"libraryId": is("a"), "deleted": operator("isTrue")}}),
        json!({"condition": {"seriesStatus": is("hiatus")}}),
        json!({"condition": {"ageRating": {"operator": "is", "value": "old"}}}),
    ] {
        let response = send(&server, None, Method::POST, LIST, Some(&body)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
    }
    let anonymous = server.request(Method::POST, LIST).json(&all).send().await;
    let anonymous = anonymous.expect("List request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn metadata_conditions_filter_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = three_series(directory.path()).await;
    let (_, alpha) = book(&pool, "a1").await;
    let (_, gamma) = book(&pool, "g1").await;
    let alpha_metadata = json!({
        "title": "Élan_100%", "titleSort": "Zeta", "publisher": "Pub", "language": "en",
        "genres": ["Action"], "tags": ["Fantasy"], "sharingLabels": ["kids"], "ageRating": 12,
        "status": "HIATUS", "totalBookCount": 2,
    });
    edit_series(&server, &alpha, &alpha_metadata).await;
    edit_series(
        &server,
        &gamma,
        &json!({"ageRating": 16, "totalBookCount": 5}),
    )
    .await;
    let others = vec!["Beta", "Gamma"];
    let string =
        |field: &str, op: &str, value: &str| json!({field: {"operator": op, "value": value}});
    let age = |op: &str, value: i32| json!({"ageRating": {"operator": op, "value": value}});

    assert_matches(
        &server,
        vec![
            (string("title", "contains", "LAN_1"), vec!["Alpha"]),
            (string("title", "doesNotBeginWith", "e"), others.clone()),
            (string("titleSort", "is", "zeta"), vec!["Alpha"]),
            (json!({"publisher": is("PUB")}), vec!["Alpha"]),
            (json!({"language": is_not("EN")}), others.clone()),
            (json!({"genre": is("action")}), vec!["Alpha"]),
            (json!({"genre": operator("isNull")}), others.clone()),
            (json!({"tag": is("fantasy")}), vec!["Alpha"]),
            (json!({"tag": is("book tag")}), vec!["Beta"]),
            (json!({"tag": operator("isNotNull")}), vec!["Alpha", "Beta"]),
            (json!({"tag": is_not("Book Tag")}), vec!["Alpha", "Gamma"]),
            (json!({"sharingLabel": is("KIDS")}), vec!["Alpha"]),
            (json!({"sharingLabel": operator("isNull")}), others.clone()),
            (age("is", 12), vec!["Alpha"]),
            (age("isNot", 12), others.clone()),
            (age("greaterThan", 12), vec!["Alpha", "Gamma"]),
            (age("lessThan", 12), vec!["Alpha"]),
            (json!({"ageRating": operator("isNull")}), vec!["Beta"]),
            (json!({"seriesStatus": is("HIATUS")}), vec!["Alpha"]),
            (json!({"seriesStatus": is_not("HIATUS")}), others.clone()),
            (json!({"complete": operator("isTrue")}), vec!["Alpha"]),
            (json!({"complete": operator("isFalse")}), vec!["Gamma"]),
            (
                json!({"author": {"operator": "is", "value": {"name": "ann", "role": "WRITER"}}}),
                vec!["Beta"],
            ),
            (
                json!({"author": {"operator": "isNot", "value": {"name": "Ann"}}}),
                vec!["Alpha", "Gamma"],
            ),
            (
                json!({"notableEntity": {"operator": "is", "value": {"kind": "character"}}}),
                vec!["Beta"],
            ),
            (
                json!({"releaseDate": {"operator": "after", "dateTime": "2020-01-01T00:00:00Z"}}),
                vec!["Beta"],
            ),
            (
                json!({"releaseDate": operator("isNull")}),
                vec!["Alpha", "Gamma"],
            ),
            (
                json!({"allOf": [age("greaterThan", 10), {"tag": operator("isNull")}]}),
                vec!["Gamma"],
            ),
            (
                json!({"anyOf": [age("is", 16), {"genre": is("action")}]}),
                vec!["Alpha", "Gamma"],
            ),
        ],
    )
    .await;
}

#[tokio::test]
async fn state_conditions_filter_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = three_series(directory.path()).await;
    let library = library["id"].as_str().expect("Library ID");
    for name in ["a1", "a2"] {
        let (id, _) = book(&pool, name).await;
        mark(&server, &id, json!({"completed": true})).await;
    }
    let (b1, _) = book(&pool, "b1").await;
    mark(&server, &b1, json!({"page": 1})).await;
    let all = vec!["Alpha", "Beta", "Gamma"];

    assert_matches(
        &server,
        vec![
            (json!({"readStatus": is("READ")}), vec!["Alpha"]),
            (json!({"readStatus": is("IN_PROGRESS")}), vec!["Beta"]),
            (json!({"readStatus": is("UNREAD")}), vec!["Gamma"]),
            (json!({"readStatus": is_not("READ")}), vec!["Beta", "Gamma"]),
            (
                json!({"readStatus": is_not("IN_PROGRESS")}),
                vec!["Alpha", "Gamma"],
            ),
            (
                json!({"readStatus": is_not("UNREAD")}),
                vec!["Alpha", "Beta"],
            ),
            (json!({"libraryId": is(library)}), all.clone()),
            (json!({"libraryId": is_not(library)}), vec![]),
            (json!({"deleted": operator("isFalse")}), all.clone()),
            (json!({"deleted": operator("isTrue")}), vec![]),
            (json!({"oneShot": operator("isFalse")}), all.clone()),
        ],
    )
    .await;
    let by_read = search(&server, None, "?sort=readDate,desc", &json!({})).await;
    assert_eq!(names(&by_read), ["Beta", "Alpha", "Gamma"]);
    let counts = &by_read["content"][1];
    assert_eq!(counts["booksReadCount"], 2);
    assert_eq!(counts["booksUnreadCount"], 0);
}

#[tokio::test]
async fn collection_conditions_order_by_collection_number() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = three_series(directory.path()).await;
    let (_, alpha) = book(&pool, "a1").await;
    let (_, gamma) = book(&pool, "g1").await;
    let body = json!({"name": "Mixed", "seriesIds": [gamma, alpha]});
    let response = send(
        &server,
        None,
        Method::POST,
        "/api/v1/collections",
        Some(&body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let collection: Value = response.json().await.expect("Collection");
    let collection = collection["id"].as_str().expect("Collection ID");
    let in_collection = json!({"condition": {"collectionId": is(collection)}});

    let ordered = search(&server, None, "?sort=collection.number", &in_collection).await;
    assert_eq!(names(&ordered), ["Gamma", "Alpha"]);
    assert_eq!(ordered["sort"]["sorted"], true);
    let reversed = search(
        &server,
        None,
        "?sort=collection.number,desc",
        &in_collection,
    )
    .await;
    assert_eq!(names(&reversed), ["Alpha", "Gamma"]);
    let outside = json!({"collectionId": is_not(collection)});
    assert_eq!(matching(&server, &outside).await, ["Beta"]);

    // Series outside the collection have no number, which sorts first.
    let any = json!([{"collectionId": is(collection)}, {"libraryId": is_not("x")}]);
    let either = json!({"condition": {"anyOf": any}});
    let page = search(&server, None, "?sort=collection.number&size=2", &either).await;
    assert_eq!(names(&page), ["Beta", "Gamma"]);
    assert_eq!(page["totalElements"], 3);

    let unsorted = search(&server, None, "?sort=collection.number", &json!({})).await;
    assert_eq!(unsorted["sort"]["sorted"], false);
}

#[tokio::test]
async fn users_list_only_series_they_may_see() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = three_series(directory.path()).await;
    let (_, beta) = book(&pool, "b1").await;
    rate(&server, &beta, 18).await;
    let sixteen = json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    create_user(&server, "restricted@example.com", sixteen).await;
    let unshared = json!({"sharedLibraries": {"all": false, "libraryIds": []}});
    create_user(&server, "unshared@example.com", unshared).await;
    let restricted = Some("restricted@example.com");

    let seen = search(&server, restricted, "?size=1&sort=name", &json!({})).await;
    assert_eq!(names(&seen), ["Alpha"]);
    assert_eq!(seen["totalElements"], 2);
    assert_eq!(
        seen["content"][0]["url"], "",
        "only admins see directory paths"
    );
    let beta_only = json!({"condition": {"tag": is("book tag")}});
    let hidden = search(&server, restricted, "", &beta_only).await;
    assert_eq!(hidden["totalElements"], 0);

    let nothing = search(&server, Some("unshared@example.com"), "", &json!({})).await;
    assert_eq!(nothing["totalElements"], 0);
    assert_eq!(names(&nothing), Vec::<&str>::new());
}
