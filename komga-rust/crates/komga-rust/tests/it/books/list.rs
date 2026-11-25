//! `POST /api/v1/books/list` (`BookController.getBooks`) with search conditions.

use reqwest::{Method, RequestBuilder, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::library_with_books;
use super::read_progress::mark;
use super::reading::access::create_reader;
use super::reading::{fetch, pages_book};
use crate::read_lists::rate;
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

const LIST: &str = "/api/v1/books/list";

/// Two series in one library: `Alpha` holds a1–a3 and `Beta` b1–b2, numbered from 1.
pub(super) async fn two_series(directory: &std::path::Path) -> (TestServer, Value, SqlitePool) {
    let books = [
        "Alpha/a1.cbz",
        "Alpha/a2.cbz",
        "Alpha/a3.cbz",
        "Beta/b1.cbz",
        "Beta/b2.cbz",
    ]
    .map(|path| (path, pages_book()));
    library_with_books(directory, &books).await
}

async fn page_of(request: RequestBuilder, body: &Value) -> Value {
    let response = request.json(body).send().await.expect("List request");
    assert_eq!(response.status(), StatusCode::OK, "{body}");
    openapi::checked(Method::POST, response).await
}

async fn search(server: &TestServer, query: &str, body: &Value) -> Value {
    page_of(server.admin(Method::POST, &format!("{LIST}{query}")), body).await
}

fn names(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|book| book["name"].as_str().expect("Book name"))
        .collect()
}

/// Names of the books matching `condition`, by name.
async fn matching(server: &TestServer, condition: Value) -> Vec<String> {
    let page = search(
        server,
        "?unpaged=true&sort=name",
        &json!({"condition": condition}),
    )
    .await;
    names(&page).into_iter().map(str::to_owned).collect()
}

fn is(value: &str) -> Value {
    json!({"operator": "is", "value": value})
}

fn is_not(value: &str) -> Value {
    json!({"operator": "isNot", "value": value})
}

pub(super) async fn edit_metadata(server: &TestServer, book_id: &str, body: Value) {
    let response = server
        .admin(Method::PATCH, &format!("/api/v1/books/{book_id}/metadata"))
        .json(&body)
        .send()
        .await
        .expect("Metadata edit");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn pages_count_and_sort_every_book() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, _) = two_series(directory.path()).await;
    let all = json!({});

    let first = search(&server, "?size=2&sort=name", &all).await;
    assert_eq!(names(&first), ["a1", "a2"]);
    assert_eq!(first["totalElements"], 5);
    assert_eq!(first["totalPages"], 3);
    assert_eq!(first["sort"]["sorted"], true);
    let last = search(&server, "?size=2&page=2&sort=name", &all).await;
    assert_eq!(names(&last), ["b2"]);
    assert_eq!(last["last"], true);
    let past = search(&server, "?size=2&page=5&sort=name", &all).await;
    assert_eq!(names(&past), Vec::<&str>::new());
    assert_eq!(past["totalElements"], 5);

    let by_number = search(&server, "?sort=metadata.numberSort,desc&sort=name", &all).await;
    assert_eq!(names(&by_number), ["a3", "a2", "b2", "a1", "b1"]);
    let unknown = search(&server, "?sort=unknown", &all).await;
    assert_eq!(unknown["sort"]["sorted"], false);
    assert_eq!(unknown["totalElements"], 5);
    let unpaged = search(&server, "?unpaged=true&sort=series,desc&sort=name", &all).await;
    assert_eq!(names(&unpaged), ["b1", "b2", "a1", "a2", "a3"]);
    assert_eq!(unpaged["size"], 20);
    let empty_any = json!({"condition": {"anyOf": []}});
    assert_eq!(search(&server, "", &empty_any).await["totalElements"], 5);

    for (body, status) in [
        (None, StatusCode::BAD_REQUEST),
        (
            Some(json!({"condition": {"title": {"operator": "nope", "value": "a"}}})),
            StatusCode::BAD_REQUEST,
        ),
        (
            Some(json!({"condition": {"seriesId": is("a"), "deleted": {"operator": "isTrue"}}})),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let mut request = server.admin(Method::POST, LIST);
        if let Some(body) = &body {
            request = request.json(body);
        }
        let response = request.send().await.expect("List request");
        assert_eq!(response.status(), status, "{body:?}");
    }
    let anonymous = fetch(server.request(Method::POST, LIST).json(&all)).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn text_conditions_ignore_accents_and_case() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = two_series(directory.path()).await;
    let (a2, _) = book(&pool, "a2").await;
    edit_metadata(
        &server,
        &a2,
        json!({
            "title": "Élan_100%",
            "tags": ["Fantasy"],
            "authors": [{"name": "Ann", "role": "writer"}],
            "notableEntities": [{"kind": "character", "name": "Bob"}],
        }),
    )
    .await;
    let others = ["a1", "a3", "b1", "b2"];
    let title =
        |operator: &str, value: &str| json!({"title": {"operator": operator, "value": value}});

    for (condition, expected) in [
        (title("contains", "LAN_1"), vec!["a2"]),
        (title("beginsWith", "_"), vec![]),
        (title("endsWith", "0%"), vec!["a2"]),
        (title("doesNotContain", "elan"), others.to_vec()),
        (title("doesNotBeginWith", "e"), others.to_vec()),
        (title("doesNotEndWith", "%"), others.to_vec()),
        (title("is", "elan_100%"), vec!["a2"]),
        (title("isNot", "ELAN_100%"), others.to_vec()),
        (json!({"tag": is("fantasy")}), vec!["a2"]),
        (json!({"tag": is_not("FANTASY")}), others.to_vec()),
        (json!({"tag": {"operator": "isNull"}}), others.to_vec()),
        (json!({"tag": {"operator": "isNotNull"}}), vec!["a2"]),
        (
            json!({"author": {"operator": "is", "value": {"name": "ann"}}}),
            vec!["a2"],
        ),
        (
            json!({"author": {"operator": "is", "value": {"role": "WRITER"}}}),
            vec!["a2"],
        ),
        (
            json!({"author": {"operator": "is", "value": {"name": "ann", "role": "inker"}}}),
            vec![],
        ),
        (
            json!({"author": {"operator": "isNot", "value": {"name": "Ann"}}}),
            others.to_vec(),
        ),
        (
            json!({"notableEntity": {"operator": "is", "value": {"name": "bob", "kind": "Character"}}}),
            vec!["a2"],
        ),
    ] {
        assert_eq!(
            matching(&server, condition.clone()).await,
            expected,
            "{condition}"
        );
    }
}

#[tokio::test]
async fn dates_numbers_and_combinations_filter_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = two_series(directory.path()).await;
    let (a2, alpha) = book(&pool, "a2").await;
    edit_metadata(&server, &a2, json!({"releaseDate": "2020-01-02"})).await;
    let others = ["a1", "a3", "b1", "b2"];
    let date = |operator: &str, date_time: &str| json!({"releaseDate": {"operator": operator, "dateTime": date_time}});
    let last = |operator: &str| json!({"releaseDate": {"operator": operator, "duration": "P1D"}});
    let number =
        |operator: &str, value: f32| json!({"numberSort": {"operator": operator, "value": value}});

    for (condition, expected) in [
        (date("before", "2020-01-03T00:00:00Z"), vec!["a2"]),
        (date("after", "2020-01-01T00:00:00Z"), vec!["a2"]),
        // 23:00 at UTC-2 is already 2 January in UTC.
        (date("after", "2020-01-01T23:00:00-02:00"), vec![]),
        (
            json!({"releaseDate": {"operator": "isNull"}}),
            others.to_vec(),
        ),
        (
            json!({"releaseDate": {"operator": "isNotNull"}}),
            vec!["a2"],
        ),
        (last("isInTheLast"), vec![]),
        (last("isNotInTheLast"), vec!["a2"]),
        (number("greaterThan", 2.0), vec!["a2", "a3", "b2"]),
        (number("lessThan", 1.0), vec!["a1", "b1"]),
        (number("is", 3.0), vec!["a3"]),
        (number("isNot", 1.0), vec!["a2", "a3", "b2"]),
        (json!({"seriesId": is(&alpha)}), vec!["a1", "a2", "a3"]),
        (json!({"seriesId": is_not(&alpha)}), vec!["b1", "b2"]),
        (
            json!({"allOf": [{"seriesId": is(&alpha)}, number("greaterThan", 2.0)]}),
            vec!["a2", "a3"],
        ),
        (
            json!({"anyOf": [number("is", 3.0), {"allOf": [{"seriesId": is_not(&alpha)}, number("is", 1.0)]}]}),
            vec!["a3", "b1"],
        ),
        // A poster match without fields matches every book, also within `anyOf`.
        (
            json!({"anyOf": [{"poster": {"operator": "is", "value": {}}}, number("is", 3.0)]}),
            vec!["a3"],
        ),
    ] {
        assert_eq!(
            matching(&server, condition.clone()).await,
            expected,
            "{condition}"
        );
    }
}

#[tokio::test]
async fn state_conditions_filter_books() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = two_series(directory.path()).await;
    let library = library["id"].as_str().expect("Library ID");
    let (a1, _) = book(&pool, "a1").await;
    let (b1, _) = book(&pool, "b1").await;
    assert_eq!(
        mark(&server, &a1, json!({"completed": true}))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        mark(&server, &b1, json!({"page": 1})).await.status(),
        StatusCode::NO_CONTENT
    );
    let all = ["a1", "a2", "a3", "b1", "b2"];
    let boolean = |name: &str, operator: &str| json!({name: {"operator": operator}});

    for (condition, expected) in [
        (json!({"readStatus": is("READ")}), vec!["a1"]),
        (json!({"readStatus": is("IN_PROGRESS")}), vec!["b1"]),
        (json!({"readStatus": is("UNREAD")}), vec!["a2", "a3", "b2"]),
        (
            json!({"readStatus": is_not("READ")}),
            vec!["a2", "a3", "b1", "b2"],
        ),
        (
            json!({"readStatus": is_not("IN_PROGRESS")}),
            vec!["a1", "a2", "a3", "b2"],
        ),
        (json!({"readStatus": is_not("UNREAD")}), vec!["a1", "b1"]),
        (json!({"mediaStatus": is("READY")}), all.to_vec()),
        (json!({"mediaStatus": is_not("READY")}), vec![]),
        (json!({"mediaProfile": is("DIVINA")}), all.to_vec()),
        (json!({"mediaProfile": is("EPUB")}), vec![]),
        (json!({"mediaProfile": is_not("EPUB")}), all.to_vec()),
        (boolean("deleted", "isFalse"), all.to_vec()),
        (boolean("deleted", "isTrue"), vec![]),
        (boolean("oneShot", "isFalse"), all.to_vec()),
        (boolean("oneShot", "isTrue"), vec![]),
        (json!({"libraryId": is(library)}), all.to_vec()),
        (json!({"libraryId": is_not(library)}), vec![]),
        (
            json!({"poster": {"operator": "isNot", "value": {"type": "USER_UPLOADED"}}}),
            all.to_vec(),
        ),
    ] {
        assert_eq!(
            matching(&server, condition.clone()).await,
            expected,
            "{condition}"
        );
    }
    let bad_status = json!({"condition": {"readStatus": is("read")}});
    let response = server
        .admin(Method::POST, LIST)
        .json(&bad_status)
        .send()
        .await;
    assert_eq!(
        response.expect("List request").status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn read_list_conditions_order_by_read_list_number() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = two_series(directory.path()).await;
    let (a1, _) = book(&pool, "a1").await;
    let (b2, beta) = book(&pool, "b2").await;
    let response = server
        .admin(Method::POST, "/api/v1/readlists")
        .json(&json!({"name": "Mixed", "bookIds": [b2, a1]}))
        .send()
        .await
        .expect("Read list creation");
    assert_eq!(response.status(), StatusCode::OK);
    let read_list = openapi::checked(Method::POST, response).await;
    let read_list = read_list["id"].as_str().expect("Read list ID");
    let in_list = json!({"condition": {"readListId": is(read_list)}});

    let ordered = search(&server, "?sort=readList.number", &in_list).await;
    assert_eq!(names(&ordered), ["b2", "a1"]);
    assert_eq!(ordered["sort"]["sorted"], true);
    let reversed = search(&server, "?sort=readList.number,desc", &in_list).await;
    assert_eq!(names(&reversed), ["a1", "b2"]);
    let outside = json!({"readListId": is_not(read_list)});
    assert_eq!(matching(&server, outside).await, ["a2", "a3", "b1"]);

    // Books outside the list have no number, which sorts first.
    let either =
        json!({"condition": {"anyOf": [{"readListId": is(read_list)}, {"seriesId": is(&beta)}]}});
    let page = search(&server, "?sort=readList.number&size=2", &either).await;
    assert_eq!(names(&page), ["b1", "b2"]);
    assert_eq!(page["totalElements"], 3);

    let unsorted = search(&server, "?sort=readList.number", &json!({})).await;
    assert_eq!(unsorted["sort"]["sorted"], false);
    assert_eq!(unsorted["totalElements"], 5);
}

#[tokio::test]
async fn users_list_only_books_they_may_see() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = two_series(directory.path()).await;
    let (_, beta) = book(&pool, "b1").await;
    rate(&server, &beta, 18).await;
    let restricted = create_reader(
        &server,
        "restricted@example.com",
        json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}}),
    )
    .await;
    let unshared = create_reader(
        &server,
        "unshared@example.com",
        json!({"roles": [], "sharedLibraries": {"all": false, "libraryIds": []}}),
    )
    .await;

    let all = json!({});
    let seen = page_of(
        restricted.request(&server, Method::POST, &format!("{LIST}?size=2&sort=name")),
        &all,
    )
    .await;
    assert_eq!(names(&seen), ["a1", "a2"]);
    assert_eq!(seen["totalElements"], 3);
    assert_eq!(
        seen["content"][0]["url"], "a1.cbz",
        "only admins see file paths"
    );
    let beta_books = json!({"condition": {"anyOf": [{"seriesId": is(&beta)}, {"deleted": {"operator": "isTrue"}}]}});
    let hidden = page_of(restricted.request(&server, Method::POST, LIST), &beta_books).await;
    assert_eq!(hidden["totalElements"], 0);

    let nothing = page_of(unshared.request(&server, Method::POST, LIST), &all).await;
    assert_eq!(nothing["totalElements"], 0);
    assert_eq!(names(&nothing), Vec::<&str>::new());
}
