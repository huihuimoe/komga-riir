//! The ISBN of a barcode on the last or first pages of a book (`IsbnBarcodeProvider`).

use reqwest::{Method, StatusCode};
use serde_json::json;

use super::{PNG, books, library, rescan, write};
use crate::libraries::{admin_json, route};
use crate::support::cbz::{self, STORED};
use crate::support::db::content::book;

const ISBN_PAGE: &str = "komga-image/tests/fixtures/barcode/page_384.jpg";
const NOT_ISBN_PAGE: &str = "komga-image/tests/fixtures/barcode/ean13_not_isbn.png";

/// A book of `pages`, in order, each a fixture path.
fn book_of(pages: &[&str]) -> Vec<u8> {
    let contents: Vec<Vec<u8>> = pages.iter().map(|path| cbz::fixture(path)).collect();
    let names: Vec<String> = pages
        .iter()
        .enumerate()
        .map(|(i, path)| format!("{i:02}.{}", path.rsplit('.').next().expect("Extension")))
        .collect();
    let members: Vec<(&str, u16, &[u8])> = names
        .iter()
        .zip(&contents)
        .map(|(name, content)| (name.as_str(), STORED, content.as_slice()))
        .collect();
    cbz::zip(&members)
}

async fn isbn(pool: &sqlx::SqlitePool, name: &str) -> String {
    let (id, _) = book(pool, name).await;
    books::fields(pool, &id).await.5
}

#[tokio::test]
async fn a_barcode_isbn_is_read_from_the_last_then_the_first_pages() {
    let directory = komga_test_support::tempdir().expect("Data directory");
    // Seven pages: the ISBN is only on the first one, after a barcode that is no ISBN last.
    let first = [ISBN_PAGE, PNG, PNG, PNG, PNG, PNG, NOT_ISBN_PAGE];
    write(directory.path(), "Series/first.cbz", &book_of(&first));
    // The ISBN on page 4 of 7 is neither among the last nor the first three pages.
    let middle = [PNG, PNG, PNG, ISBN_PAGE, PNG, PNG, PNG];
    write(directory.path(), "Series/middle.cbz", &book_of(&middle));
    let (server, library, pool) = library(directory.path()).await;
    assert_eq!(isbn(&pool, "first").await, "9782811632397");
    assert_eq!(isbn(&pool, "middle").await, "");

    admin_json(
        &server,
        Method::PATCH,
        &route(&library),
        Some(&json!({"importBarcodeIsbn": false})),
        StatusCode::NO_CONTENT,
    )
    .await;
    write(
        directory.path(),
        "Series/middle.cbz",
        &book_of(&[ISBN_PAGE]),
    );
    rescan(&server, directory.path(), &library).await;
    assert_eq!(
        isbn(&pool, "middle").await,
        "",
        "the library imports no barcode ISBN"
    );
    server.stop().await;
}
