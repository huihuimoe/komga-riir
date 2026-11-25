//! `GET /api/v1/readlists`.

use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use std::fs;
use std::path::Path;

use komga_rust::config::{AppConfig, CliArgs, ConfigError};

use super::{Fixture, PNG, READ_LISTS, create_list, create_user, fixture, rate, send};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::{TestServer, openapi};

/// Creates the read lists `Alpha` (`b1`), `álpha` (`a2`), `beta` (`a1`) and `Gamma` (`a1`, `b1`),
/// which a byte order would sort `Alpha`, `Gamma`, `beta`, `álpha`.
async fn create_lists(f: &Fixture) {
    let lists = [
        ("Gamma", vec![&f.a1, &f.b1]),
        ("beta", vec![&f.a1]),
        ("álpha", vec![&f.a2]),
        ("Alpha", vec![&f.b1]),
    ];
    for (name, books) in lists {
        let body = json!({"name": name, "bookIds": books});
        let response = send(&f.server, None, Method::POST, READ_LISTS, Some(&body)).await;
        assert_eq!(response.status(), StatusCode::OK, "{name}");
    }
}

async fn page(f: &Fixture, email: Option<&str>, query: &str) -> Value {
    let path = format!("{READ_LISTS}?{query}");
    let response = send(&f.server, email, Method::GET, &path, None).await;
    assert_eq!(response.status(), StatusCode::OK, "{query}");
    openapi::checked(Method::GET, response).await
}

/// The configuration of the data directory under the environment `env`.
fn config(directory: &Path, env: &[(&str, String)]) -> Result<AppConfig, ConfigError> {
    let args = CliArgs {
        config_dir: Some(directory.to_path_buf()),
        ..CliArgs::default()
    };
    AppConfig::resolve(&args, &env.iter().cloned().collect())
}

/// Stops `server` and starts one over the same data under the environment `env`.
async fn restart(server: TestServer, directory: &Path, env: &[(&str, String)]) -> TestServer {
    server.stop().await;
    TestServer::start_with(config(directory, env).expect("Configuration")).await
}

fn names(page: &Value) -> Vec<&str> {
    page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .map(|list| list["name"].as_str().expect("Name"))
        .collect()
}

/// The `(bookIds, filtered)` of the listed read list `name`.
fn books_of<'a>(page: &'a Value, name: &str) -> (&'a Value, &'a Value) {
    let list = page["content"]
        .as_array()
        .expect("Page content")
        .iter()
        .find(|list| list["name"] == name)
        .expect("Listed read list");
    (&list["bookIds"], &list["filtered"])
}

#[tokio::test]
async fn admins_page_read_lists_by_unicode_name_order() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    create_lists(&f).await;

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
    assert_eq!(
        books_of(&in_a, "Gamma"),
        (&json!([&f.a1, &f.b1]), &json!(false)),
        "the filter keeps every book"
    );

    let path = format!("{READ_LISTS}?unpaged=maybe");
    let response = send(&f.server, None, Method::GET, &path, None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    f.server.stop().await;
}

#[tokio::test]
async fn users_page_only_read_lists_with_books_they_may_see() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let f = fixture(directory.path()).await;
    create_lists(&f).await;
    let library_a = json!({"sharedLibraries": {"all": false, "libraryIds": [&f.library_a]}});
    create_user(&f.server, "a@example.com", library_a).await;
    let allow_12 = json!({"ageRestriction": {"age": 12, "restriction": "ALLOW_ONLY"}});
    create_user(&f.server, "twelve@example.com", allow_12).await;
    rate(&f.server, &f.series_a, 18).await;
    rate(&f.server, &f.series_b, 6).await;

    let a = Some("a@example.com");
    let seen = page(&f, a, "").await;
    assert_eq!(names(&seen), ["álpha", "beta", "Gamma"]);
    assert_eq!(seen["totalElements"], 3);
    assert_eq!(books_of(&seen, "Gamma"), (&json!([&f.a1]), &json!(true)));
    let elsewhere = page(&f, a, "library_id=unshared").await;
    assert_eq!(
        (names(&elsewhere).len(), &elsewhere["totalElements"]),
        (0, &json!(0))
    );

    let first = page(&f, Some("twelve@example.com"), "size=1").await;
    assert_eq!(names(&first), ["Alpha"]);
    assert_eq!(
        (&first["totalElements"], &first["totalPages"]),
        (&json!(2), &json!(2))
    );
    let all = page(&f, Some("twelve@example.com"), "unpaged=true").await;
    assert_eq!(names(&all), ["Alpha", "Gamma"]);
    assert_eq!(books_of(&all, "Gamma"), (&json!([&f.b1]), &json!(true)));
    f.server.stop().await;
}

#[tokio::test]
async fn names_sort_by_the_library_locale_or_else_the_global_one() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let mut f = fixture(directory.path()).await;
    let lists = [
        ("z", vec![&f.a1]),
        ("ä", vec![&f.a2]),
        ("a10", vec![&f.a1]),
        ("a2", vec![&f.a1, &f.b1]),
        ("y", vec![&f.b1]),
        ("一", vec![&f.b1]),
        ("二", vec![&f.b1]),
    ];
    for (name, books) in lists {
        let body = json!({"name": name, "bookIds": books});
        let response = send(&f.server, None, Method::POST, READ_LISTS, Some(&body)).await;
        assert_eq!(response.status(), StatusCode::OK, "{name}");
    }
    let library_b: String = sqlx::query_scalar(r#"SELECT "ID" FROM "LIBRARY" WHERE "ID" <> ?"#)
        .bind(&f.library_a)
        .fetch_one(&f.pool)
        .await
        .expect("Library B");
    let both = format!("library_id={}&library_id={library_b}", f.library_a);
    let only_a = format!("library_id={}", f.library_a);
    let only_b = format!("library_id={library_b}");

    // Without settings, every result sorts by the root locale: an accent
    // only breaks ties, digits compare by value, and Han by radical.
    assert_eq!(
        names(&page(&f, None, "").await),
        ["ä", "a2", "a10", "y", "z", "一", "二"]
    );
    assert_eq!(
        names(&page(&f, None, &only_a).await),
        ["ä", "a2", "a10", "z"]
    );

    let sort = [
        ("KOMGA_SORT_LOCALE", "zh-u-co-pinyin".to_owned()),
        ("KOMGA_SORT_LIBRARYLOCALES", format!("{}=sv", f.library_a)),
    ];
    f.server = restart(f.server, directory.path(), &sort).await;

    // Library A alone sorts in Swedish, where `ä` follows `z`.
    assert_eq!(
        names(&page(&f, None, &only_a).await),
        ["a2", "a10", "z", "ä"]
    );
    // Library B has no locale of its own, and several libraries share the global one, which
    // puts Han first and in pinyin order.
    assert_eq!(
        names(&page(&f, None, &only_b).await),
        ["二", "一", "a2", "y"]
    );
    for query in ["", both.as_str()] {
        assert_eq!(
            names(&page(&f, None, query).await),
            ["二", "一", "ä", "a2", "a10", "y", "z"],
            "{query}"
        );
    }
    assert_eq!(
        names(&page(&f, None, &format!("{only_a}&sort=name,desc")).await),
        ["ä", "z", "a10", "a2"]
    );

    create_user(
        &f.server,
        "a@example.org",
        json!({"sharedLibraries": {"all": false, "libraryIds": [f.library_a]}}),
    )
    .await;
    // A user limited to library A reads it alone.
    assert_eq!(
        names(&page(&f, Some("a@example.org"), "").await),
        ["a2", "a10", "z", "ä"]
    );
    f.server.stop().await;
}

/// The names of every page of `query`, two lists at a time.
async fn paged_names(f: &Fixture, query: &str) -> Vec<String> {
    let mut all = Vec::new();
    for number in 0.. {
        let page = page(f, None, &format!("{query}&size=2&page={number}")).await;
        all.extend(names(&page).into_iter().map(str::to_owned));
        if page["last"] == true {
            break;
        }
    }
    all
}

/// What a sort locale must leave alone: the pages of book `book` and the books of read list
/// `list`, each in order.
async fn stored_orders(f: &Fixture, book: &str, list: &str) -> (Value, Value) {
    let mut orders = Vec::new();
    for path in [
        format!("/api/v1/books/{book}/pages"),
        format!("{READ_LISTS}/{list}/books?unpaged=true"),
    ] {
        let response = send(&f.server, None, Method::GET, &path, None).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        orders.push(openapi::checked(Method::GET, response).await);
    }
    let books = orders.pop().expect("Books")["content"].take();
    (orders.pop().expect("Pages"), books)
}

#[tokio::test]
async fn a_new_sort_locale_reorders_names_alone_and_a_rejected_one_changes_nothing() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let png = cbz::fixture(PNG);
    let pages = [
        ("ö.png", STORED, &*png),
        ("z.png", STORED, &*png),
        ("o.png", STORED, &*png),
    ];
    fs::create_dir_all(directory.path().join("a/SA")).expect("Series directory");
    fs::write(directory.path().join("a/SA/pages.cbz"), cbz::zip(&pages)).expect("Book file");
    let mut f = fixture(directory.path()).await;
    let (paged_book, _) = book(&f.pool, "pages").await;
    // `1` and `01` are equal in every locale, so only their code points order them.
    for name in ["z", "ö", "o", "1", "01"] {
        create_list(&f, name, &[&f.a1]).await;
    }
    let manual = create_list(&f, "manual", &[&f.b1, &f.a2, &f.a1]).await;
    let library = format!("library_id={}", f.library_a);
    let before = stored_orders(&f, &paged_book, &manual).await;
    let root = ["01", "1", "manual", "o", "ö", "z"];
    assert_eq!(paged_names(&f, &library).await, root);

    // An unsupported locale stops the server before it opens its data.
    let unsupported = [("KOMGA_SORT_LIBRARYLOCALES", format!("{}=zh", f.library_a))];
    f.server.stop().await;
    assert!(config(directory.path(), &unsupported).is_err());
    f.server = TestServer::start(directory.path()).await;
    assert_eq!(paged_names(&f, &library).await, root);

    let swedish = [("KOMGA_SORT_LIBRARYLOCALES", format!("{}=sv", f.library_a))];
    f.server = restart(f.server, directory.path(), &swedish).await;
    let sorted = ["01", "1", "manual", "o", "z", "ö"];
    assert_eq!(paged_names(&f, &library).await, sorted);
    let unpaged = page(&f, None, &format!("{library}&unpaged=true")).await;
    assert_eq!(names(&unpaged), sorted);
    let descending = paged_names(&f, &format!("{library}&sort=name,desc")).await;
    assert!(descending.iter().rev().eq(&sorted), "{descending:?}");
    assert_eq!(stored_orders(&f, &paged_book, &manual).await, before);
    f.server.stop().await;
}
