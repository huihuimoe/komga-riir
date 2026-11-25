//! `GET /api/v1/collections/{id}/series` (`SeriesCollectionController.getSeriesByCollectionId`).

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::read_lists::{create_user, rate, send};
use crate::series::details::mark;
use crate::series::list::{edit_series, names, three_series};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

/// The series IDs of `Alpha`, `Beta` and `Gamma`.
async fn series_ids(pool: &SqlitePool) -> [String; 3] {
    let mut ids = Vec::new();
    for book_name in ["a1", "b1", "g1"] {
        ids.push(book(pool, book_name).await.1);
    }
    ids.try_into().expect("Three series")
}

/// Creates a collection of `series` and returns its ID.
async fn create_collection(
    server: &TestServer,
    name: &str,
    ordered: bool,
    series: &[&str],
) -> String {
    let body = json!({"name": name, "ordered": ordered, "seriesIds": series});
    let response = send(
        server,
        None,
        Method::POST,
        "/api/v1/collections",
        Some(&body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let collection: Value = response.json().await.expect("Collection");
    collection["id"].as_str().expect("Collection ID").to_owned()
}

/// The page of series of collection `id` for `query`, as `email` sees it, or the failure status.
async fn series_of(
    server: &TestServer,
    email: Option<&str>,
    id: &str,
    query: &str,
) -> Result<Value, StatusCode> {
    let path = format!("/api/v1/collections/{id}/series{query}");
    let response = send(server, email, Method::GET, &path, None).await;
    match response.status() {
        StatusCode::OK => Ok(openapi::checked(Method::GET, response).await),
        status => Err(status),
    }
}

/// Names of the series of collection `id` the admin gets for the filter `query`.
async fn filtered(server: &TestServer, id: &str, query: &str) -> Vec<String> {
    let page = series_of(server, None, id, query).await.expect(query);
    names(&page).into_iter().map(str::to_owned).collect()
}

#[tokio::test]
async fn ordered_collections_page_by_number_others_by_title_sort() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = three_series(directory.path()).await;
    let [alpha, beta, gamma] = series_ids(&pool).await;
    let members = [gamma.as_str(), alpha.as_str(), beta.as_str()];
    let ordered = create_collection(&server, "Ordered", true, &members).await;
    let unordered = create_collection(&server, "Unordered", false, &members).await;

    let all = series_of(&server, None, &ordered, "?sort=name")
        .await
        .expect("Series");
    assert_eq!(
        names(&all),
        ["Gamma", "Alpha", "Beta"],
        "the requested sort is ignored"
    );
    assert_eq!(all["sort"]["sorted"], true);
    assert_eq!(all["totalElements"], 3);
    assert_eq!(all["content"][0]["booksCount"], 3);
    let second = series_of(&server, None, &ordered, "?size=2&page=1")
        .await
        .expect("Series");
    assert_eq!(names(&second), ["Beta"]);
    assert_eq!(second["totalPages"], 2);
    let past = series_of(&server, None, &ordered, "?size=2&page=5")
        .await
        .expect("Series");
    assert_eq!(names(&past), Vec::<&str>::new());
    assert_eq!(past["totalElements"], 3);
    let unpaged = series_of(&server, None, &ordered, "?unpaged=true&size=1")
        .await
        .expect("Series");
    assert_eq!(names(&unpaged), ["Gamma", "Alpha", "Beta"]);

    let by_title = series_of(&server, None, &unordered, "")
        .await
        .expect("Series");
    assert_eq!(names(&by_title), ["Alpha", "Beta", "Gamma"]);
    edit_series(&server, &gamma, &json!({"titleSort": "Aardvark"})).await;
    let resorted = series_of(&server, None, &unordered, "?sort=name,desc")
        .await
        .expect("Series");
    assert_eq!(names(&resorted), ["Gamma", "Alpha", "Beta"]);

    let bad_flag = series_of(&server, None, &ordered, "?deleted=maybe").await;
    assert_eq!(bad_flag.unwrap_err(), StatusCode::BAD_REQUEST);
    let bad_status = series_of(&server, None, &ordered, "?status=DONE").await;
    assert_eq!(bad_status.unwrap_err(), StatusCode::BAD_REQUEST);
    let unknown = series_of(&server, None, "unknown", "").await;
    assert_eq!(unknown.unwrap_err(), StatusCode::NOT_FOUND);
    let path = format!("/api/v1/collections/{ordered}/series");
    let anonymous = server
        .request(Method::GET, &path)
        .send()
        .await
        .expect("Request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn filter_parameters_match_any_of_their_values() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) = three_series(directory.path()).await;
    let [alpha, beta, gamma] = series_ids(&pool).await;
    let id = create_collection(&server, "All", false, &[&alpha, &beta, &gamma]).await;
    let alpha_metadata = json!({
        "status": "ENDED", "publisher": "Pub", "language": "fr", "genres": ["Drama"],
        "tags": ["Series Tag"], "totalBookCount": 2,
    });
    edit_series(&server, &alpha, &alpha_metadata).await;
    edit_series(
        &server,
        &gamma,
        &json!({"status": "HIATUS", "totalBookCount": 9}),
    )
    .await;
    rate(&server, &beta, 18).await;
    let (b1, _) = book(&pool, "b1").await;
    mark(&server, &b1, json!({"completed": true})).await;
    let library_id = library["id"].as_str().expect("Library ID");

    let cases = [
        (
            format!("?library_id={library_id}"),
            vec!["Alpha", "Beta", "Gamma"],
        ),
        ("?library_id=other".to_owned(), vec![]),
        ("?status=ENDED,HIATUS".to_owned(), vec!["Alpha", "Gamma"]),
        (
            "?status=ENDED&status=ONGOING".to_owned(),
            vec!["Alpha", "Beta"],
        ),
        ("?publisher=pub".to_owned(), vec!["Alpha"]),
        ("?language=fr".to_owned(), vec!["Alpha"]),
        ("?genre=drama".to_owned(), vec!["Alpha"]),
        (
            "?tag=book%20tag&tag=series%20tag".to_owned(),
            vec!["Alpha", "Beta"],
        ),
        ("?age_rating=18".to_owned(), vec!["Beta"]),
        ("?age_rating=none".to_owned(), vec!["Alpha", "Gamma"]),
        (
            "?age_rating=18,none".to_owned(),
            vec!["Alpha", "Beta", "Gamma"],
        ),
        ("?release_year=2020".to_owned(), vec!["Beta"]),
        ("?release_year=2019,2021".to_owned(), vec![]),
        (
            "?release_year=never".to_owned(),
            vec!["Alpha", "Beta", "Gamma"],
        ),
        ("?release_year=2147483647".to_owned(), vec![]),
        ("?read_status=READ".to_owned(), vec!["Beta"]),
        (
            "?read_status=UNREAD,IN_PROGRESS".to_owned(),
            vec!["Alpha", "Gamma"],
        ),
        ("?author=Ann,Writer".to_owned(), vec!["Beta"]),
        ("?author=Ann,penciller".to_owned(), vec![]),
        ("?author=no-role".to_owned(), vec!["Alpha", "Beta", "Gamma"]),
        ("?complete=true".to_owned(), vec!["Alpha"]),
        ("?complete=false".to_owned(), vec!["Gamma"]),
        ("?deleted=false".to_owned(), vec!["Alpha", "Beta", "Gamma"]),
        ("?deleted=true".to_owned(), vec![]),
        ("?status=ENDED&publisher=other".to_owned(), vec![]),
    ];
    for (query, expected) in cases {
        assert_eq!(filtered(&server, &id, &query).await, expected, "{query}");
    }
}

#[tokio::test]
async fn users_see_only_series_they_may_see() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = three_series(directory.path()).await;
    let [alpha, beta, gamma] = series_ids(&pool).await;
    rate(&server, &beta, 18).await;
    let all = create_collection(&server, "All", true, &[&beta, &alpha, &gamma]).await;
    let adult = create_collection(&server, "Adult", true, &[&beta]).await;
    let sixteen = json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}});
    create_user(&server, "restricted@example.com", sixteen).await;
    let restricted = Some("restricted@example.com");

    let seen = series_of(&server, restricted, &all, "?size=1")
        .await
        .expect("Series");
    assert_eq!(names(&seen), ["Alpha"]);
    assert_eq!(seen["totalElements"], 2);
    assert_eq!(
        seen["content"][0]["url"], "",
        "only admins see directory paths"
    );
    let admin = series_of(&server, None, &all, "").await.expect("Series");
    assert_ne!(admin["content"][0]["url"], "");
    let hidden = series_of(&server, restricted, &adult, "").await;
    assert_eq!(hidden.unwrap_err(), StatusCode::NOT_FOUND);
}
