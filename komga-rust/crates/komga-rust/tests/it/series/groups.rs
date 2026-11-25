//! `POST /api/v1/series/list/alphabetical-groups` (`SeriesController.getSeriesAlphabeticalGroups`).

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::list::{edit_series, three_series};
use crate::read_lists::{create_user, rate, send};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

const GROUPS: &str = "/api/v1/series/list/alphabetical-groups";

/// The groups `email` (else the admin) gets for `body`, as `(group, count)`.
async fn groups(server: &TestServer, email: Option<&str>, body: &Value) -> Vec<(String, u64)> {
    let response = send(server, email, Method::POST, GROUPS, Some(body)).await;
    assert_eq!(response.status(), StatusCode::OK, "{body}");
    let groups: Value = openapi::checked(Method::POST, response).await;
    let groups = groups.as_array().expect("Group list");
    groups
        .iter()
        .map(|group| {
            let name = group["group"].as_str().expect("Group name").to_owned();
            (name, group["count"].as_u64().expect("Group count"))
        })
        .collect()
}

/// `totalElements` of `series/list` for `email` (else the admin) and `body`.
async fn listed(server: &TestServer, email: Option<&str>, body: &Value) -> u64 {
    let response = send(
        server,
        email,
        Method::POST,
        "/api/v1/series/list",
        Some(body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "{body}");
    let page: Value = response.json().await.expect("Series page");
    page["totalElements"].as_u64().expect("Total elements")
}

fn group(name: &str, count: u64) -> (String, u64) {
    (name.to_owned(), count)
}

#[tokio::test]
async fn series_count_by_first_character_of_title_sort() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = three_series(directory.path()).await;
    let library = library["id"].as_str().expect("Library ID");
    let (_, beta) = book(&pool, "b1").await;
    let (_, gamma) = book(&pool, "g1").await;
    edit_series(&server, &beta, &json!({"titleSort": "about"})).await;
    // SQLite `lower` folds only ASCII letters.
    edit_series(&server, &gamma, &json!({"titleSort": "Élan"})).await;
    rate(&server, &beta, 18).await;
    let sixteen = json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    create_user(&server, "restricted@example.com", sixteen).await;
    let restricted = Some("restricted@example.com");

    let all = json!({});
    assert_eq!(
        groups(&server, None, &all).await,
        [group("a", 2), group("É", 1)]
    );
    assert_eq!(
        groups(&server, restricted, &all).await,
        [group("a", 1), group("É", 1)]
    );
    let tagged = json!({"condition": {"tag": {"operator": "is", "value": "book tag"}}});
    assert_eq!(groups(&server, None, &tagged).await, [group("a", 1)]);
    assert_eq!(groups(&server, restricted, &tagged).await, []);
    let elsewhere = json!({"condition": {"libraryId": {"operator": "isNot", "value": library}}});
    assert_eq!(groups(&server, None, &elsewhere).await, []);

    for (email, body) in [(None, &all), (restricted, &all), (None, &tagged)] {
        let grouped: u64 = groups(&server, email, body)
            .await
            .iter()
            .map(|(_, n)| n)
            .sum();
        assert_eq!(grouped, listed(&server, email, body).await, "{body}");
    }

    let bad = json!({"condition": {"seriesStatus": {"operator": "is", "value": "hiatus"}}});
    let response = send(&server, None, Method::POST, GROUPS, Some(&bad)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let anonymous = server.request(Method::POST, GROUPS).json(&all).send().await;
    assert_eq!(
        anonymous.expect("Groups request").status(),
        StatusCode::UNAUTHORIZED
    );
}
