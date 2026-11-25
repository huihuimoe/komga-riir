//! The deprecated `GET /api/v1/series` and `GET /api/v1/series/alphabetical-groups`
//! (`SeriesController.getSeriesDeprecated/getSeriesAlphabeticalGroupsDeprecated`).

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::list::{edit_series, names, three_series};
use crate::read_lists::{create_user, rate, send};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

const SERIES: &str = "/api/v1/series";
const GROUPS: &str = "/api/v1/series/alphabetical-groups";
const RESTRICTED: &str = "restricted@example.com";

async fn get(server: &TestServer, email: Option<&str>, path: &str) -> Value {
    let response = send(server, email, Method::GET, path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    openapi::checked(Method::GET, response).await
}

/// Names of the series for `query`, by name.
async fn series(server: &TestServer, query: &str) -> Vec<String> {
    let page = get(
        server,
        None,
        &format!("{SERIES}?unpaged=true&sort=name&{query}"),
    )
    .await;
    names(&page).into_iter().map(str::to_owned).collect()
}

/// `group:count` of each alphabetical group for `query`.
async fn groups(server: &TestServer, email: Option<&str>, query: &str) -> Vec<String> {
    let groups = get(server, email, &format!("{GROUPS}?{query}")).await;
    groups
        .as_array()
        .expect("Groups")
        .iter()
        .map(|group| {
            format!(
                "{}:{}",
                group["group"].as_str().expect("Group"),
                group["count"]
            )
        })
        .collect()
}

async fn create_collection(server: &TestServer, series_ids: &[&str]) -> String {
    let body = json!({"name": "Mixed", "seriesIds": series_ids});
    let path = "/api/v1/collections";
    let response = send(server, None, Method::POST, path, Some(&body)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let collection: Value = response.json().await.expect("Collection");
    collection["id"].as_str().expect("Collection ID").to_owned()
}

#[tokio::test]
async fn query_parameters_filter_series() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = three_series(directory.path()).await;
    let (_, alpha) = book(&pool, "a1").await;
    let (_, gamma) = book(&pool, "g1").await;
    let metadata = json!({"publisher": "Pub", "language": "en", "genres": ["Action"],
        "sharingLabels": ["kids"], "ageRating": 12});
    edit_series(&server, &alpha, &metadata).await;
    let collection = create_collection(&server, &[&gamma, &alpha]).await;
    let library_id = library["id"].as_str().expect("Library ID");
    let all = ["Alpha", "Beta", "Gamma"];

    for (query, expected) in [
        (format!("library_id={library_id}"), all.to_vec()),
        ("library_id=0".to_owned(), vec![]),
        (
            format!("collection_id={collection}"),
            vec!["Alpha", "Gamma"],
        ),
        ("status=ONGOING".to_owned(), all.to_vec()),
        ("status=ENDED,ABANDONED".to_owned(), vec![]),
        ("read_status=UNREAD".to_owned(), all.to_vec()),
        ("publisher=Pub".to_owned(), vec!["Alpha"]),
        ("language=en".to_owned(), vec!["Alpha"]),
        ("genre=Action".to_owned(), vec!["Alpha"]),
        ("tag=Book Tag".to_owned(), vec!["Beta"]),
        ("sharing_label=kids".to_owned(), vec!["Alpha"]),
        ("age_rating=12".to_owned(), vec!["Alpha"]),
        ("age_rating=None".to_owned(), vec!["Beta", "Gamma"]),
        ("release_year=2020".to_owned(), vec!["Beta"]),
        ("release_year=x".to_owned(), all.to_vec()),
        ("author=Ann,writer".to_owned(), vec!["Beta"]),
        ("complete=true".to_owned(), vec![]),
        // Neither without a total book count.
        ("complete=false".to_owned(), vec![]),
        ("oneshot=false&deleted=false".to_owned(), all.to_vec()),
        ("deleted=true".to_owned(), vec![]),
        ("search_regex=^a,title".to_owned(), vec!["Alpha"]),
        ("search_regex=MM,TITLE_SORT".to_owned(), vec!["Gamma"]),
        ("search_regex=^a,name".to_owned(), all.to_vec()),
        ("search_regex=^a".to_owned(), all.to_vec()),
    ] {
        assert_eq!(series(&server, &query).await, expected, "{query}");
    }
    for query in ["status=BAD", "deleted=maybe", "search_regex=[,title"] {
        let response = send(
            &server,
            None,
            Method::GET,
            &format!("{SERIES}?{query}"),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
    }

    let unsorted = get(&server, None, SERIES).await;
    assert_eq!(unsorted["totalElements"], 3);
    assert_eq!(unsorted["sort"]["sorted"], false);
    let numbered = format!("{SERIES}?collection_id={collection}&sort=collection.number");
    let numbered = get(&server, None, &numbered).await;
    assert_eq!(names(&numbered), ["Gamma", "Alpha"]);
    assert_eq!(numbered["sort"]["sorted"], true);
    let ignored = get(&server, None, &format!("{SERIES}?sort=collection.number")).await;
    assert_eq!(ignored["sort"]["sorted"], false);
}

#[tokio::test]
async fn groups_count_the_series_of_the_same_parameters() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = three_series(directory.path()).await;
    assert_eq!(groups(&server, None, "").await, ["a:1", "b:1", "g:1"]);
    assert_eq!(
        groups(&server, None, "search_regex=^a,title").await,
        ["a:1"]
    );
    assert_eq!(groups(&server, None, "tag=Book Tag&page=3").await, ["b:1"]);
    assert_eq!(
        groups(&server, None, "library_id=0").await,
        Vec::<String>::new()
    );
    let response = send(
        &server,
        None,
        Method::GET,
        &format!("{GROUPS}?complete=x"),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let (_, beta) = book(&pool, "b1").await;
    rate(&server, &beta, 18).await;
    let sixteen = json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    create_user(&server, RESTRICTED, sixteen).await;
    assert_eq!(groups(&server, Some(RESTRICTED), "").await, ["a:1", "g:1"]);
    let seen = get(&server, Some(RESTRICTED), &format!("{SERIES}?sort=name")).await;
    assert_eq!(names(&seen), ["Alpha", "Gamma"]);
    assert_eq!(
        seen["content"][0]["url"], "",
        "only admins see directory paths"
    );
    let anonymous = server
        .request(Method::GET, GROUPS)
        .send()
        .await
        .expect("Request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}
