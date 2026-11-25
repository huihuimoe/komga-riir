//! `GET /api/v1/collections`.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use super::{COLLECTIONS, Fixture, create_user, fixture, rate, send};
use crate::support::openapi;

/// Creates the collections `Alpha` (`b`), `álpha` (`a2`), `beta` (`a1`) and `Gamma` (`a1`, `b`),
/// which a byte order would sort `Alpha`, `Gamma`, `beta`, `álpha`.
async fn create_collections(f: &Fixture) {
    let collections = [
        ("Gamma", vec![&f.a1, &f.b]),
        ("beta", vec![&f.a1]),
        ("álpha", vec![&f.a2]),
        ("Alpha", vec![&f.b]),
    ];
    for (name, series) in collections {
        let body = json!({"name": name, "seriesIds": series});
        let response = send(&f.server, None, Method::POST, COLLECTIONS, Some(&body)).await;
        assert_eq!(response.status(), StatusCode::OK, "{name}");
    }
}

async fn page(f: &Fixture, email: Option<&str>, query: &str) -> Value {
    let path = format!("{COLLECTIONS}?{query}");
    let response = send(&f.server, email, Method::GET, &path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "{query}");
    openapi::checked(Method::GET, response).await
}

fn names(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|collection| collection["name"].as_str().expect("Name"))
        .collect()
}

/// The `(seriesIds, filtered)` of the listed collection `name`.
fn series_of<'a>(page: &'a Value, name: &str) -> (&'a Value, &'a Value) {
    let collection = page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .find(|collection| collection["name"] == name)
        .expect("Listed collection");
    (&collection["seriesIds"], &collection["filtered"])
}

#[tokio::test]
async fn admins_page_collections_by_unicode_name_order() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    create_collections(&f).await;

    let all = page(&f, None, "").await;
    assert_eq!(names(&all), ["Alpha", "álpha", "beta", "Gamma"]);
    assert_eq!(
        (&all["totalElements"], &all["size"]),
        (&json!(4), &json!(20))
    );
    assert_eq!(all["sort"]["sorted"], true);
    let second = page(&f, None, "size=2&page=1").await;
    assert_eq!(names(&second), ["beta", "Gamma"]);
    assert_eq!(
        (&second["totalElements"], &second["totalPages"]),
        (&json!(4), &json!(2))
    );
    let descending = page(&f, None, "sort=name,desc").await;
    assert_eq!(names(&descending), ["Gamma", "beta", "álpha", "Alpha"]);
    let unknown = page(&f, None, "sort=unknown").await;
    assert_eq!(
        (&unknown["totalElements"], &unknown["sort"]["sorted"]),
        (&json!(4), &json!(false))
    );
    let unpaged = page(&f, None, "unpaged=true&size=1").await;
    assert_eq!(names(&unpaged).len(), 4);
    assert_eq!(
        (&unpaged["size"], &unpaged["totalPages"]),
        (&json!(20), &json!(1))
    );

    let in_a = page(&f, None, &format!("library_id={}", f.library_a)).await;
    assert_eq!(names(&in_a), ["álpha", "beta", "Gamma"]);
    assert_eq!(in_a["totalElements"], 3);
    let every_series = (&json!([&f.a1, &f.b]), &json!(false));
    assert_eq!(
        series_of(&in_a, "Gamma"),
        every_series,
        "the filter keeps every series"
    );

    let path = format!("{COLLECTIONS}?unpaged=maybe");
    let response = send(&f.server, None, Method::GET, &path, None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    f.server.stop().await;
}

#[tokio::test]
async fn users_page_only_collections_with_series_they_may_see() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    create_collections(&f).await;
    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.com", library_a).await;
    let allow_12 = json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}});
    create_user(&f.server, "twelve@example.com", allow_12).await;
    rate(&f.server, &f.a1, 18).await;
    rate(&f.server, &f.a2, 18).await;
    rate(&f.server, &f.b, 6).await;

    let a = Some("a@example.com");
    let seen = page(&f, a, "").await;
    assert_eq!(names(&seen), ["álpha", "beta", "Gamma"]);
    assert_eq!(seen["totalElements"], 3);
    assert_eq!(series_of(&seen, "Gamma"), (&json!([&f.a1]), &json!(true)));
    let elsewhere = page(&f, a, "library_id=unshared").await;
    assert_eq!(
        (names(&elsewhere).len(), &elsewhere["totalElements"]),
        (0, &json!(0))
    );

    let twelve = Some("twelve@example.com");
    let first = page(&f, twelve, "size=1").await;
    assert_eq!(names(&first), ["Alpha"]);
    assert_eq!(
        (&first["totalElements"], &first["totalPages"]),
        (&json!(2), &json!(2))
    );
    let all = page(&f, twelve, "unpaged=true").await;
    assert_eq!(names(&all), ["Alpha", "Gamma"]);
    assert_eq!(series_of(&all, "Gamma"), (&json!([&f.b]), &json!(true)));
    f.server.stop().await;
}
