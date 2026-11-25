//! `AnalyzeBook`: ZIP books analyzed after scans and on request.

#[cfg(unix)]
mod versions;

use std::fs::{self, File};
use std::time::{Duration, SystemTime};

use reqwest::{Method, StatusCode};
use serde_json::json;

use super::read_progress::{mark, progress, row, series_progress};
use super::reading::access::create_reader;
use super::{COMIC_INFO, GIF, JPEG, PNG, analyze, library_with_books};
use crate::libraries::{admin_json, route};
use crate::support::cbz::{self, STORED, UNSUPPORTED, ZSTD};
use crate::support::db::content::{self, book, files, media, pages};
use crate::support::tasks::{queue_kotlin_analysis, tasks, wait_for_scan_work};
use crate::support::{ADMIN_EMAIL, TestServer, openapi};

/// A CBZ whose members are out of natural order, with a directory, two non-page members, and a
/// member whose compression method cannot be read.
fn mixed_book() -> Vec<u8> {
    let (png, gif, jpeg) = (cbz::fixture(PNG), cbz::fixture(GIF), cbz::fixture(JPEG));
    cbz::zip(&[
        ("dir/", STORED, b""),
        ("page 10.png", STORED, &png),
        ("notes.txt", STORED, b"some notes\n"),
        ("page 2.jpg", STORED, &jpeg),
        ("ComicInfo.xml", STORED, &cbz::fixture(COMIC_INFO)),
        ("bad.png", UNSUPPORTED, &png),
        ("Page 1.gif", STORED, &gif),
    ])
}

fn size(path: &str) -> Option<i64> {
    i64::try_from(cbz::fixture(path).len()).ok()
}

#[tokio::test]
async fn a_scanned_cbz_is_analyzed_into_ordered_pages_and_files() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) =
        library_with_books(directory.path(), &[("Series/book.cbz", mixed_book())]).await;
    let (book_id, series_id) = book(&pool, "book").await;

    assert_eq!(
        media(&pool, &book_id).await,
        (
            "READY".to_owned(),
            Some("application/zip".to_owned()),
            Some("ERR_1007 [bad.png]".to_owned()),
            3
        )
    );
    let page = |name: &str, media_type: &str, side: Option<(i32, i32)>, path| {
        let (width, height) = side.unzip();
        (
            name.to_owned(),
            media_type.to_owned(),
            width,
            height,
            String::new(),
            size(path),
        )
    };
    assert_eq!(
        pages(&pool, &book_id).await,
        [
            page("Page 1.gif", "image/gif", Some((48, 20)), GIF),
            page("page 2.jpg", "image/jpeg", Some((3, 2)), JPEG),
            page("page 10.png", "image/png", Some((48, 20)), PNG),
        ]
    );
    let stored_files = vec![
        ("bad.png".to_owned(), None, size(PNG)),
        (
            "ComicInfo.xml".to_owned(),
            Some("application/xml".to_owned()),
            size(COMIC_INFO),
        ),
        (
            "notes.txt".to_owned(),
            Some("text/plain".to_owned()),
            Some(11),
        ),
    ];
    assert_eq!(files(&pool, &book_id).await, stored_files);
    // A ready analysis starts the metadata refresh chain that ends with the series aggregation;
    // `books::thumbnails` covers the thumbnail generation it also queues.
    assert_eq!(
        content::aggregated_since(&pool, &[]).await,
        [series_id.as_str()]
    );
    let aggregations = content::aggregation_dates(&pool).await;

    // Without dimension analysis, a requested analysis stores no sizes.
    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"analyzeDimensions": false})),
        StatusCode::NO_CONTENT,
    )
    .await;
    analyze(&server, &book_id).await;
    wait_for_scan_work(directory.path()).await;
    let sizes: Vec<(Option<i32>, Option<i32>)> = pages(&pool, &book_id)
        .await
        .into_iter()
        .map(|row| (row.2, row.3))
        .collect();
    assert_eq!(sizes, [(None, None); 3]);
    assert_eq!(files(&pool, &book_id).await, stored_files);
    assert_eq!(
        content::aggregated_since(&pool, &aggregations).await,
        [series_id]
    );
    server.stop().await;
}

#[tokio::test]
async fn analysis_failures_are_stored_by_cause() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let no_pages = cbz::zip(&[("ComicInfo.xml", STORED, &cbz::fixture(COMIC_INFO))]);
    let zstd = cbz::zip(&[("page.png", ZSTD, &cbz::fixture(PNG))]);
    // One entry more than opens.
    let names: Vec<String> = (0..=100_000).map(|index| index.to_string()).collect();
    let crowded: Vec<(&str, u16, &[u8])> = names
        .iter()
        .map(|name| (name.as_str(), STORED, &[][..]))
        .collect();
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("no-pages.cbz", no_pages),
            ("broken.cbz", b"PK\x03\x04 is not a whole archive".to_vec()),
            ("crowded.cbz", cbz::zip(&crowded)),
            ("text.cbz", b"plain text, not a book".to_vec()),
            ("zstd.cbz", zstd),
        ],
    )
    .await;
    let status = |status: &str, media_type: Option<&str>, comment: &str| {
        (
            status.to_owned(),
            media_type.map(str::to_owned),
            Some(comment.to_owned()),
            0,
        )
    };
    for (name, expected) in [
        (
            "no-pages",
            status("ERROR", Some("application/zip"), "ERR_1006"),
        ),
        (
            "broken",
            status("ERROR", Some("application/zip"), "ERR_1008"),
        ),
        (
            "crowded",
            status("ERROR", Some("application/zip"), "ERR_1008"),
        ),
        (
            "text",
            status("UNSUPPORTED", Some("text/plain"), "ERR_1001"),
        ),
    ] {
        let (id, _) = book(&pool, name).await;
        assert_eq!(media(&pool, &id).await, expected, "{name}");
        assert!(pages(&pool, &id).await.is_empty(), "{name}");
        assert!(files(&pool, &id).await.is_empty(), "{name}");
    }
    // A missing codec class fails Kotlin's whole analysis: the task fails, the media stays.
    let (zstd_id, _) = book(&pool, "zstd").await;
    assert_eq!(
        media(&pool, &zstd_id).await,
        ("UNKNOWN".to_owned(), None, None, 0)
    );
    let failed = tasks(directory.path()).await;
    assert!(
        failed
            .iter()
            .any(|row| row.0 == format!("ANALYZE_BOOK_{zstd_id}")
                && row.3.contains("\"eraFailed\":1")),
        "{failed:?}"
    );

    // A file that cannot be opened or no longer exists fails before its type is known.
    let root = directory.path().join("comics");
    fs::remove_file(root.join("text.cbz")).expect("Remove book");
    let (text_id, _) = book(&pool, "text").await;
    analyze(&server, &text_id).await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let (no_pages_id, _) = book(&pool, "no-pages").await;
        let locked = root.join("no-pages.cbz");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("Lock book");
        analyze(&server, &no_pages_id).await;
        wait_for_scan_work(directory.path()).await;
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).expect("Unlock book");
        assert_eq!(
            media(&pool, &no_pages_id).await,
            ("ERROR".to_owned(), None, Some("ERR_1000".to_owned()), 0)
        );
    }
    wait_for_scan_work(directory.path()).await;
    assert_eq!(
        media(&pool, &text_id).await,
        ("ERROR".to_owned(), None, Some("ERR_1018".to_owned()), 0)
    );
    server.stop().await;
}

#[tokio::test]
async fn analysis_requests_need_an_admin_and_an_existing_book() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) =
        library_with_books(directory.path(), &[("book.cbz", mixed_book())]).await;
    let (book_id, _) = book(&pool, "book").await;
    let reader = ("reader@example.com", "reader-password");
    admin_json(
        &server,
        Method::POST,
        "/api/v2/users",
        Some(&json!({"email": reader.0, "password": reader.1})),
        StatusCode::CREATED,
    )
    .await;
    let path = format!("/api/v1/books/{book_id}/analyze");
    for (request, status) in [
        (
            server.request(Method::POST, &path),
            StatusCode::UNAUTHORIZED,
        ),
        (
            server
                .request(Method::POST, "/api/v1/books/missing/analyze")
                .basic_auth(reader.0, Some(reader.1)),
            StatusCode::FORBIDDEN,
        ),
        (
            server.admin(Method::POST, "/api/v1/books/missing/analyze"),
            StatusCode::NOT_FOUND,
        ),
    ] {
        assert_eq!(request.send().await.expect("Request").status(), status);
    }
    assert!(
        !tasks(directory.path())
            .await
            .iter()
            .any(|row| row.0.starts_with("ANALYZE_BOOK_")),
        "rejected requests queue nothing"
    );
    let response = server
        .admin(Method::POST, &path)
        .send()
        .await
        .expect("Request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    openapi::checked(Method::POST, response).await;
    server.stop().await;
}

#[tokio::test]
async fn a_reanalysis_with_another_page_count_moves_read_progress() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, library, pool) =
        library_with_books(directory.path(), &[("Series/book.cbz", mixed_book())]).await;
    let (book_id, series_id) = book(&pool, "book").await;
    let reader = create_reader(&server, "reader@example.com", json!({"roles": []})).await;
    mark(&server, &book_id, json!({"page": 2})).await;
    let completed = reader
        .request(
            &server,
            Method::PATCH,
            &format!("/api/v1/books/{book_id}/read-progress"),
        )
        .json(&json!({"completed": true}));
    assert_eq!(
        completed.send().await.expect("Request").status(),
        StatusCode::NO_CONTENT
    );

    // Two pages instead of three.
    let path = directory.path().join("comics/Series/book.cbz");
    let png = cbz::fixture(PNG);
    fs::write(
        &path,
        cbz::zip(&[("1.png", STORED, &png), ("2.png", STORED, &png)]),
    )
    .expect("Replace book");
    File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_modified(SystemTime::now() + Duration::from_secs(60)))
        .expect("Book time");
    let scan = server.admin(Method::POST, &format!("{}/scan?deep=true", route(&library)));
    assert_eq!(
        scan.send().await.expect("Scan").status(),
        StatusCode::ACCEPTED
    );
    wait_for_scan_work(directory.path()).await;

    assert_eq!(media(&pool, &book_id).await.3, 2);
    assert_eq!(
        progress(&pool, &book_id).await,
        [
            row(ADMIN_EMAIL, 1, false),
            row("reader@example.com", 2, true)
        ]
    );
    assert_eq!(
        series_progress(&pool, &series_id).await,
        [
            (ADMIN_EMAIL.to_owned(), 0, 1),
            ("reader@example.com".to_owned(), 1, 0)
        ]
    );
    server.stop().await;
}

#[tokio::test]
async fn an_analysis_left_queued_by_kotlin_runs_after_a_restart() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[("book.cbz", b"not a book yet".to_vec())],
    )
    .await;
    let (book_id, series_id) = book(&pool, "book").await;
    assert_eq!(media(&pool, &book_id).await.0, "UNSUPPORTED");
    server.stop().await;

    fs::write(directory.path().join("comics/book.cbz"), mixed_book()).expect("Replace book");
    queue_kotlin_analysis(directory.path(), &book_id, &series_id).await;

    let server = TestServer::start(directory.path()).await;
    wait_for_scan_work(directory.path()).await;
    assert_eq!(
        media(&pool, &book_id).await,
        (
            "READY".to_owned(),
            Some("application/zip".to_owned()),
            Some("ERR_1007 [bad.png]".to_owned()),
            3
        )
    );
    server.stop().await;
}

#[tokio::test]
async fn a_series_analysis_request_reanalyzes_each_book() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let books = [
        ("Series/a.cbz", mixed_book()),
        ("Series/b.cbz", mixed_book()),
    ];
    let (server, _, pool) = library_with_books(directory.path(), &books).await;
    let (a_id, series_id) = book(&pool, "a").await;
    let (b_id, _) = book(&pool, "b").await;
    let reader = ("reader@example.com", "reader-password");
    admin_json(
        &server,
        Method::POST,
        "/api/v2/users",
        Some(&json!({"email": reader.0, "password": reader.1})),
        StatusCode::CREATED,
    )
    .await;
    let path = format!("/api/v1/series/{series_id}/analyze");
    for (request, status) in [
        (
            server.request(Method::POST, &path),
            StatusCode::UNAUTHORIZED,
        ),
        (
            server
                .request(Method::POST, &path)
                .basic_auth(reader.0, Some(reader.1)),
            StatusCode::FORBIDDEN,
        ),
        // Kotlin does not look the series up: an unknown one has no books to analyze.
        (
            server.admin(Method::POST, "/api/v1/series/missing/analyze"),
            StatusCode::ACCEPTED,
        ),
        (
            server
                .request(Method::POST, "/api/v1/libraries/missing/analyze")
                .basic_auth(reader.0, Some(reader.1)),
            StatusCode::FORBIDDEN,
        ),
        (
            server.admin(Method::POST, "/api/v1/libraries/missing/analyze"),
            StatusCode::ACCEPTED,
        ),
    ] {
        assert_eq!(request.send().await.expect("Request").status(), status);
    }
    assert!(
        !tasks(directory.path())
            .await
            .iter()
            .any(|row| row.0.starts_with("ANALYZE_BOOK_")),
        "rejected and empty requests queue nothing"
    );

    let root = directory.path().join("comics/Series");
    fs::remove_file(root.join("a.cbz")).expect("Remove book");
    fs::remove_file(root.join("b.cbz")).expect("Remove book");
    let response = server
        .admin(Method::POST, &path)
        .send()
        .await
        .expect("Request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    openapi::checked(Method::POST, response).await;
    wait_for_scan_work(directory.path()).await;
    for id in [a_id, b_id] {
        assert_eq!(
            media(&pool, &id).await,
            ("ERROR".to_owned(), None, Some("ERR_1018".to_owned()), 0)
        );
    }
    server.stop().await;
}
