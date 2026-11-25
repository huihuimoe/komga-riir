//! RAR books (`RarExtractor`) from scan to analysis, pages and cover.

use reqwest::header::CONTENT_TYPE;
use reqwest::{Method, StatusCode};

use super::reading::{Fetched, fetch};
use super::thumbnails::assert_jpeg;
use super::{PNG, library_with_books};
use crate::support::TestServer;
use crate::support::cbz;
use crate::support::db::content::{book, files, media, pages};
use crate::support::tasks::wait_for_generations;

const RAR: &str = "application/x-rar-compressed";
const RAR4: &str = "application/x-rar-compressed; version=4";
const RAR5: &str = "application/x-rar-compressed; version=5";
const JPEG: &str = "komga-image/tests/fixtures/photo.jpg";

fn rar(name: &str) -> Vec<u8> {
    cbz::fixture(&format!("komga-archive/tests/fixtures/rar/{name}"))
}

async fn page(server: &TestServer, book_id: &str, number: u32) -> Fetched {
    fetch(server.admin(
        Method::GET,
        &format!("/api/v1/books/{book_id}/pages/{number}"),
    ))
    .await
}

#[tokio::test]
async fn rar4_books_are_analyzed_and_read_from_scan() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/plain.cbr", rar("rar4.rar")),
            ("Series/solid.cbr", rar("rar4-solid.rar")),
            ("Series/names.cbr", rar("rar4-names.rar")),
        ],
    )
    .await;

    // Komga's own samples: three 3108-byte PNG pages, solid or not.
    for name in ["plain", "solid"] {
        let (book_id, _) = book(&pool, name).await;
        assert_eq!(
            media(&pool, &book_id).await,
            ("READY".to_owned(), Some(RAR4.to_owned()), None, 3),
            "{name}"
        );
        let names: Vec<String> = pages(&pool, &book_id)
            .await
            .into_iter()
            .map(|page| page.0)
            .collect();
        assert_eq!(
            names,
            ["komga-1.png", "komga-2.png", "komga-3.png"],
            "{name}"
        );
        let last = page(&server, &book_id, 3).await;
        assert_eq!(last.status, StatusCode::OK, "{name}");
        assert_eq!(last.header(&CONTENT_TYPE), "image/png", "{name}");
        assert_eq!(last.body.len(), 3108, "{name}");
    }

    // Names as junrar decodes them, in natural order; the directory is not a member.
    let (book_id, _) = book(&pool, "names").await;
    assert_eq!(
        media(&pool, &book_id).await,
        ("READY".to_owned(), Some(RAR4.to_owned()), None, 3)
    );
    let page_row = |name: &str, media_type: &str, size| {
        (
            name.to_owned(),
            media_type.to_owned(),
            Some(48),
            Some(20),
            String::new(),
            Some(size),
        )
    };
    assert_eq!(
        pages(&pool, &book_id).await,
        [
            page_row("café-10.png", "image/png", 251),
            page_row("sub\\page-2.jpg", "image/jpeg", 1807),
            page_row("第1話.png", "image/png", 251),
        ]
    );
    assert_eq!(
        files(&pool, &book_id).await,
        // `<ComicInfo/>` without an XML declaration, which Tika reads as text.
        [(
            "ComicInfo.xml".to_owned(),
            Some("text/plain".to_owned()),
            Some(13)
        )]
    );
    for (number, fixture) in [(1, PNG), (2, JPEG), (3, PNG)] {
        let served = page(&server, &book_id, number).await;
        assert_eq!(served.status, StatusCode::OK, "page {number}");
        assert!(served.body == cbz::fixture(fixture), "page {number} bytes");
    }

    // The cover is the first page.
    wait_for_generations(directory.path()).await;
    let thumbnail =
        fetch(server.admin(Method::GET, &format!("/api/v1/books/{book_id}/thumbnail"))).await;
    assert_jpeg(&thumbnail, (48, 20));
}

/// Page names and sizes in page order.
async fn page_names(pool: &sqlx::SqlitePool, book_id: &str) -> Vec<(String, Option<i64>)> {
    pages(pool, book_id)
        .await
        .into_iter()
        .map(|page| (page.0, page.5))
        .collect()
}

#[tokio::test]
async fn rar5_rar7_and_unversioned_books_are_analyzed_and_read_from_scan() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/plain.cbr", rar("rar5.rar")),
            ("Series/solid.cbr", rar("rar5-solid.rar")),
            ("Series/rar7.cbr", rar("rar7.rar")),
            ("Series/names.cbr", rar("rar5-names.rar")),
            ("Series/unversioned.cbr", rar("rar-unversioned.rar")),
            ("Series/unknown.cbr", rar("rar-unknown.rar")),
            ("Series/rar14.cbr", rar("rar14-prefixed.rar")),
        ],
    )
    .await;
    let ready =
        |media_type: &str, count| ("READY".to_owned(), Some(media_type.to_owned()), None, count);

    let (book_id, _) = book(&pool, "plain").await;
    assert_eq!(media(&pool, &book_id).await, ready(RAR5, 1));
    assert_eq!(page(&server, &book_id, 1).await.body.len(), 3108);
    let (book_id, _) = book(&pool, "solid").await;
    assert_eq!(media(&pool, &book_id).await, ready(RAR5, 3));
    assert_eq!(page(&server, &book_id, 3).await.body.len(), 3108);

    // RAR 7 is RAR 5 to Tika; its solid stream decodes for each page read alone.
    let (rar7_id, _) = book(&pool, "rar7").await;
    assert_eq!(media(&pool, &rar7_id).await, ready(RAR5, 2));
    assert_eq!(
        page_names(&pool, &rar7_id).await,
        [
            ("r7/page-1.bmp".to_owned(), Some(2934)),
            ("r7/page-2.png".to_owned(), Some(251))
        ]
    );
    let first = page(&server, &rar7_id, 1).await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.header(&CONTENT_TYPE), "image/bmp");
    assert_eq!(first.body.len(), 2934);

    // junrar names: `;N` for an older file version, malformed UTF-8 replaced. A `Rar!` prefix
    // leaves Tika's generic RAR type, and junrar still finds the archive after it.
    for (name, media_type) in [("names", RAR5), ("unversioned", RAR)] {
        let (book_id, _) = book(&pool, name).await;
        assert_eq!(media(&pool, &book_id).await, ready(media_type, 3), "{name}");
        assert_eq!(
            page_names(&pool, &book_id).await,
            [
                ("caf\u{fffd}.png".to_owned(), Some(251)),
                ("page.png".to_owned(), Some(1719)),
                ("page.png;1".to_owned(), Some(251)),
            ],
            "{name}"
        );
        for (number, size) in [(2, 1719), (3, 251)] {
            let served = page(&server, &book_id, number).await;
            assert_eq!(served.body.len(), size, "{name} page {number}");
        }
    }

    // Archives junrar cannot open.
    for name in ["unknown", "rar14"] {
        let (book_id, _) = book(&pool, name).await;
        assert_eq!(
            media(&pool, &book_id).await,
            (
                "ERROR".to_owned(),
                Some(RAR.to_owned()),
                Some("ERR_1008".to_owned()),
                0
            ),
            "{name}"
        );
    }

    wait_for_generations(directory.path()).await;
    let thumbnail =
        fetch(server.admin(Method::GET, &format!("/api/v1/books/{rar7_id}/thumbnail"))).await;
    assert_jpeg(&thumbnail, (48, 20));
}

#[tokio::test]
async fn encrypted_volume_damaged_and_oversized_books_are_classified() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    let (server, _, pool) = library_with_books(
        directory.path(),
        &[
            ("Series/h4.cbr", rar("rar4-header-encrypted.rar")),
            ("Series/h5.cbr", rar("rar5-header-encrypted.rar")),
            ("Series/f4.cbr", rar("rar4-file-encrypted.rar")),
            ("Series/f5.cbr", rar("rar5-file-encrypted.rar")),
            ("Series/v4.cbr", rar("rar4-volume.part1.rar")),
            ("Series/v5.cbr", rar("rar5-volume.part01.rar")),
            ("Series/c4.cbr", rar("rar4-corrupt.rar")),
            ("Series/c5.cbr", rar("rar5-corrupt.rar")),
            ("Series/t4.cbr", rar("rar4-truncated.rar")),
            ("Series/t5.cbr", rar("rar5-truncated.rar")),
            ("Series/oversized.cbr", rar("rar4-oversized.rar")),
            ("Series/dictionary.cbr", rar("rar5-dictionary.rar")),
        ],
    )
    .await;
    let failed = |status: &str, media_type: &str, comment: &str| {
        (
            status.to_owned(),
            Some(media_type.to_owned()),
            Some(comment.to_owned()),
            0,
        )
    };
    let expected = [
        ("h4", failed("UNSUPPORTED", RAR4, "ERR_1002")),
        ("h5", failed("UNSUPPORTED", RAR5, "ERR_1002")),
        ("f4", failed("UNSUPPORTED", RAR4, "ERR_1002")),
        ("f5", failed("UNSUPPORTED", RAR5, "ERR_1002")),
        ("v4", failed("UNSUPPORTED", RAR4, "ERR_1004")),
        ("v5", failed("UNSUPPORTED", RAR5, "ERR_1004")),
        ("t4", failed("ERROR", RAR4, "ERR_1008")),
        ("t5", failed("ERROR", RAR5, "ERR_1008")),
        // Every member of the solid stream is left unread, so there is no page.
        ("dictionary", failed("ERROR", RAR5, "ERR_1006")),
    ];
    for (name, media_row) in expected {
        let (book_id, _) = book(&pool, name).await;
        assert_eq!(media(&pool, &book_id).await, media_row, "{name}");
    }

    // A member that cannot be read is listed and kept as a file without a type.
    let partial = |media_type: &str, comment: &str| {
        (
            "READY".to_owned(),
            Some(media_type.to_owned()),
            Some(comment.to_owned()),
            2,
        )
    };
    for (name, media_type) in [("c4", RAR4), ("c5", RAR5)] {
        let (book_id, _) = book(&pool, name).await;
        assert_eq!(
            media(&pool, &book_id).await,
            partial(media_type, "ERR_1007 [sub/page-2.jpg]"),
            "{name}"
        );
        assert!(
            files(&pool, &book_id)
                .await
                .contains(&("sub/page-2.jpg".to_owned(), None, Some(1807))),
            "{name}"
        );
        assert_eq!(
            page(&server, &book_id, 2).await.status,
            StatusCode::OK,
            "{name}"
        );
    }
    let (book_id, _) = book(&pool, "oversized").await;
    assert_eq!(
        media(&pool, &book_id).await,
        partial(RAR4, "ERR_1007 [komga-2.png]")
    );
    assert_eq!(page(&server, &book_id, 2).await.body.len(), 3108);
}
