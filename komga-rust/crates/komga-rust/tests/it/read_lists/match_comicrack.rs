//! `POST /api/v1/readlists/match/comicrack`.

use std::fs;
use std::path::Path;

use reqwest::multipart::{Form, Part};
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};
use sqlx::SqlitePool;

use super::{PASSWORD, PNG, READ_LISTS, create_user, send};
use crate::libraries::create;
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;
use crate::support::tasks::wait_for_scan_work;
use crate::support::{TestServer, db, json, openapi};

const MATCH: &str = "/api/v1/readlists/match/comicrack";

/// Series `Batman` (`01`, `02`) and `Batman (2005)` (`x`, whose `ComicInfo.xml` numbers it
/// `001` and dates it 2005-03-04).
async fn server(directory: &Path) -> (TestServer, SqlitePool) {
    let png = cbz::fixture(PNG);
    let plain = cbz::zip(&[("page.png", STORED, &png)]);
    let comic_info = b"<ComicInfo><Number>001</Number><Year>2005</Year><Month>3</Month>\
        <Day>4</Day></ComicInfo>";
    let with_info = cbz::zip(&[
        ("ComicInfo.xml", STORED, comic_info),
        ("page.png", STORED, &png),
    ]);
    let books = [
        ("lib/Batman/01.cbz", &plain),
        ("lib/Batman/02.cbz", &plain),
        ("lib/Batman (2005)/x.cbz", &with_info),
    ];
    for (path, content) in books {
        let path = directory.join(path);
        fs::create_dir_all(path.parent().expect("Series directory")).expect("Series directory");
        fs::write(path, content).expect("Book file");
    }
    let server = TestServer::start_claimed(directory).await;
    create(&server, "Lib", &directory.join("lib")).await;
    wait_for_scan_work(directory).await;
    (server, db::main_pool(directory).await)
}

fn cbl(xml: &str) -> Form {
    Form::new().part("file", Part::text(xml.to_owned()).file_name("list.cbl"))
}

async fn post(server: &TestServer, email: Option<&str>, form: Form) -> Response {
    let request = match email {
        Some(email) => server
            .request(Method::POST, MATCH)
            .basic_auth(email, Some(PASSWORD)),
        None => server.admin(Method::POST, MATCH),
    };
    request.multipart(form).send().await.expect("Match request")
}

fn found(series: (&str, &str), release_date: Value, books: &[(&str, &str, &str)]) -> Value {
    let books: Vec<Value> = books
        .iter()
        .map(|(id, number, title)| json!({"bookId": id, "number": number, "title": title}))
        .collect();
    json!({
        "series": {"seriesId": series.0, "title": series.1, "releaseDate": release_date},
        "books": books,
    })
}

#[tokio::test]
async fn books_match_by_series_title_and_number_without_leading_zeros() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, pool) = server(directory.path()).await;
    let (b01, batman) = book(&pool, "01").await;
    let (b02, _) = book(&pool, "02").await;
    let (x, batman_2005) = book(&pool, "x").await;
    let existing = json!({"name": "Batman Arc", "bookIds": [&b01]});
    let response = send(&server, None, Method::POST, READ_LISTS, Some(&existing)).await;
    assert_eq!(response.status(), StatusCode::OK);

    let xml = r#"<?xml version="1.0"?>
<ReadingList><Name>BATMAN arc</Name><Books>
  <Book Series="batman" Number=" 02 " Volume="2005" Year="2005"><Id>1</Id></Book>
  <Book><Series>Batman</Series><Number>1</Number><Volume>2005</Volume></Book>
  <Book Series="Robin" Number="1" />
</Books></ReadingList>"#;
    let response = post(&server, None, cbl(xml)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = openapi::checked(Method::POST, response).await;

    // Series come in the order of the request's titles.
    let both_series = [
        found(
            (&batman_2005, "Batman (2005)"),
            json!("2005-03-04"),
            &[(&x, "001", "x")],
        ),
        found((&batman, "Batman"), Value::Null, &[(&b01, "1", "01")]),
    ];
    let batman_2 = found((&batman, "Batman"), Value::Null, &[(&b02, "2", "02")]);
    let series_2005 = ["batman (2005)", "batman"];
    assert_eq!(
        body,
        json!({
            "readListMatch": {"name": "BATMAN arc", "errorCode": "ERR_1009"},
            "requests": [
                {"request": {"series": series_2005, "number": "02"}, "matches": [batman_2]},
                {
                    "request": {"series": ["Batman (2005)", "Batman"], "number": "1"},
                    "matches": both_series,
                },
                {"request": {"series": ["Robin"], "number": "1"}, "matches": []},
            ],
            "errorCode": "",
        })
    );

    let fresh = r#"<ReadingList><Name>New</Name><Books><Book Series="Robin" Number="1"/>
        </Books></ReadingList>"#;
    let body = json(post(&server, None, cbl(fresh)).await).await;
    assert_eq!(
        body["readListMatch"],
        json!({"name": "New", "errorCode": ""})
    );
}

#[tokio::test]
async fn invalid_lists_missing_files_and_non_admins_are_rejected() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _) = server(directory.path()).await;
    let valid = r#"<ReadingList><Name>L</Name><Books><Book Series="Batman" Number="1"/>
        </Books></ReadingList>"#;
    let cases = [
        ("<ReadingList><Name>L</Name>", "ERR_1015"),
        (
            r#"<ReadingList><Books><Book Series="S" Number="1"/></Books></ReadingList>"#,
            "ERR_1030",
        ),
        (
            "<ReadingList><Name>L</Name><Books/></ReadingList>",
            "ERR_1029",
        ),
        (
            r#"<ReadingList><Name>L</Name><Books><Book Number="1"/></Books></ReadingList>"#,
            "ERR_1031",
        ),
    ];
    for (xml, code) in cases {
        let response = post(&server, None, cbl(xml)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{xml}");
        let body = json(response).await;
        assert_eq!(body["message"], code, "{xml}");
    }

    let other = Form::new().part("list", Part::text(valid).file_name("list.cbl"));
    let response = post(&server, None, other).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        json(response).await["message"],
        "Required part 'file' is not present."
    );

    create_user(&server, "reader@example.org", json!({"roles": []})).await;
    let response = post(&server, Some("reader@example.org"), cbl(valid)).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
