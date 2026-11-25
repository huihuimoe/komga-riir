use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::support::{TestServer, db, json, openapi};

/// `(series, library, age rating, sharing labels)`, seeded as rows so each case sets its rating
/// and labels without scanning files.
const SERIES: [(&str, &str, Option<i32>, &[&str]); 6] = [
    ("s-6", "lib-a", Some(6), &["kids"]),
    ("s-12", "lib-a", Some(12), &[]),
    ("s-null", "lib-a", None, &["kids"]),
    ("s-16", "lib-a", Some(16), &["adult"]),
    ("s-18", "lib-b", Some(18), &[]),
    ("s-12-dup", "lib-a", Some(12), &[]),
];

/// A claimed server over the seeded catalog; keep the directory alive while it runs.
async fn catalog_server() -> (komga_test_support::TestDir, TestServer) {
    let temp_dir = komga_test_support::tempdir().expect("Failed to create data directory");
    let server = TestServer::start_claimed(temp_dir.path()).await;
    let pool = db::main_pool(temp_dir.path()).await;
    seed_catalog(&pool).await;
    (temp_dir, server)
}

fn admin() -> (String, String) {
    (
        crate::support::ADMIN_EMAIL.to_owned(),
        crate::support::ADMIN_PASSWORD.to_owned(),
    )
}

#[tokio::test]
async fn age_ratings_apply_library_age_and_label_scope_in_sql() {
    let (_temp_dir, server) = catalog_server().await;
    let cases = [
        ("admin@example.com", None, json!([6, 12, 16, 18])),
        (
            "lib-a@example.com",
            Some(json!({"sharedLibraries": {"all": false, "libraryIds": ["lib-a"]}})),
            json!([6, 12, 16]),
        ),
        (
            "none@example.com",
            Some(json!({"sharedLibraries": {"all": false, "libraryIds": []}})),
            json!([]),
        ),
        (
            "allow12@example.com",
            Some(json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}})),
            json!([6, 12]),
        ),
        (
            "exclude16@example.com",
            Some(json!({"ageRestriction": {"age": 16, "restriction": "EXCLUDE"}})),
            json!([6, 12]),
        ),
        (
            "kids@example.com",
            Some(json!({"labelsAllow": ["Kids"]})),
            json!([6]),
        ),
        (
            "kids-or-12@example.com",
            Some(
                json!({"labelsAllow": ["kids"], "ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}}),
            ),
            json!([6, 12]),
        ),
        (
            "no-adult@example.com",
            Some(json!({"labelsExclude": ["adult"]})),
            json!([6, 12, 18]),
        ),
        (
            "kids-no-null@example.com",
            Some(
                json!({"labelsAllow": ["kids"], "ageRestriction": {"age": 1, "restriction": "EXCLUDE"}}),
            ),
            json!([]),
        ),
    ];
    for (email, access, expected) in cases {
        let auth = match access {
            Some(access) => create_user(&server, email, access).await,
            None => admin(),
        };
        let page = age_ratings(&server, &auth, "unpaged=true").await;
        assert_eq!(page["content"], expected, "{email}");
        assert_eq!(
            page["totalElements"],
            expected.as_array().map_or(0, Vec::len),
            "{email}"
        );
    }
    server.stop().await;
}

#[tokio::test]
async fn age_ratings_filter_by_library_and_collection_within_scope() {
    let (_temp_dir, server) = catalog_server().await;
    let lib_a = create_user(
        &server,
        "filter@example.com",
        json!({"sharedLibraries": {"all": false, "libraryIds": ["lib-a"]}}),
    )
    .await;
    let admin = admin();
    for (auth, query, expected) in [
        (&admin, "unpaged=yes&library_id=lib-b", json!([18])),
        (
            &admin,
            "unpaged=1&library_id=lib-a,%20lib-b",
            json!([6, 12, 16, 18]),
        ),
        (
            &admin,
            "unpaged=on&library_id=lib-b&library_id=missing",
            json!([18]),
        ),
        (&admin, "unpaged=true&collection_id=c-1", json!([12, 18])),
        (
            &admin,
            "unpaged=true&library_id=lib-a&collection_id=c-1",
            json!([6, 12, 16]),
        ),
        (&lib_a, "unpaged=true&library_id=lib-b", json!([])),
        (&lib_a, "unpaged=true&collection_id=c-1", json!([12])),
    ] {
        let page = age_ratings(&server, auth, query).await;
        assert_eq!(page["content"], expected, "{query}");
    }

    server.stop().await;
}

#[tokio::test]
async fn age_ratings_page_like_spring_page_impl() {
    let (_temp_dir, server) = catalog_server().await;
    let admin = admin();

    // Unrated series hold no value, so pages and totals cover 6, 12, 16 and 18 only.
    let first = age_ratings(&server, &admin, "page=0&size=2").await;
    assert_eq!(
        first,
        json!({
            "content": [6, 12],
            "pageable": {
                "pageNumber": 0, "pageSize": 2, "offset": 0, "paged": true, "unpaged": false,
                "sort": {"empty": false, "sorted": true, "unsorted": false}
            },
            "last": false,
            "totalPages": 2,
            "totalElements": 4,
            "size": 2,
            "number": 0,
            "sort": {"empty": false, "sorted": true, "unsorted": false},
            "first": true,
            "numberOfElements": 2,
            "empty": false
        })
    );
    let last = age_ratings(&server, &admin, "page=1&size=2").await;
    assert_eq!(last["content"], json!([16, 18]));
    assert_eq!(last["totalElements"], 4);
    assert_eq!(last["last"], true);
    assert_eq!(last["first"], false);

    let beyond = age_ratings(&server, &admin, "page=7&size=2").await;
    assert_eq!(beyond["content"], json!([]));
    assert_eq!(beyond["empty"], true);
    assert_eq!(beyond["totalElements"], 4);
    assert_eq!(beyond["number"], 7);
    assert_eq!(beyond["pageable"]["offset"], 14);

    for (query, number, size) in [
        ("", 0, 20),
        ("page=-3&size=0", 0, 20),
        ("page=abc&size=x", 0, 20),
        ("page=%20&size=5000", 0, 2000),
        ("page=1&page=0&size=3", 1, 3),
        ("unpaged=&page=1&size=3", 1, 3),
    ] {
        let page = age_ratings(&server, &admin, query).await;
        assert_eq!(
            (page["number"].clone(), page["size"].clone()),
            (json!(number), json!(size)),
            "{query}"
        );
    }

    let unpaged = age_ratings(&server, &admin, "unpaged=TRUE&page=1&size=1").await;
    assert_eq!(unpaged["content"], json!([6, 12, 16, 18]));
    assert_eq!(unpaged["pageable"]["pageSize"], 20);
    assert_eq!(unpaged["pageable"]["paged"], true);
    assert_eq!(unpaged["totalPages"], 1);

    let response = server
        .admin(Method::GET, "/api/v2/age-ratings?unpaged=maybe")
        .send()
        .await
        .expect("Invalid unpaged request failed");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = json(response).await;
    assert_eq!(error["error"], "Bad Request");
    assert_eq!(
        error["message"],
        "Method parameter 'unpaged': Failed to convert value of type 'java.lang.String' to \
         required type 'boolean'; Invalid boolean value [maybe]"
    );

    let response = server
        .request(Method::GET, "/api/v2/age-ratings")
        .send()
        .await
        .expect("Anonymous request failed");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    server.stop().await;
}

async fn seed_catalog(pool: &SqlitePool) {
    for library in ["lib-a", "lib-b"] {
        db::seed::library(pool, library).await;
    }
    for (series, library, age_rating, labels) in SERIES {
        db::seed::series(pool, series, library).await;
        db::seed::series_metadata(pool, series, age_rating, labels).await;
    }
    db::seed::collection(pool, "c-1", &["s-12", "s-18"]).await;
}

/// Creates a user with the given access fields and returns its Basic credentials.
async fn create_user(server: &TestServer, email: &str, access: Value) -> (String, String) {
    let mut body = access;
    body["email"] = json!(email);
    body["password"] = json!("secret");
    let response = server
        .admin(Method::POST, "/api/v2/users")
        .json(&body)
        .send()
        .await
        .expect("User creation failed");
    assert_eq!(response.status(), StatusCode::CREATED, "{email}");
    (email.to_owned(), "secret".to_owned())
}

async fn age_ratings(
    server: &TestServer,
    (email, password): &(String, String),
    query: &str,
) -> Value {
    let response = server
        .request(Method::GET, &format!("/api/v2/age-ratings?{query}"))
        .basic_auth(email, Some(password))
        .send()
        .await
        .expect("Age-ratings request failed");
    assert_eq!(response.status(), StatusCode::OK, "{query}");
    openapi::checked(Method::GET, response).await
}
