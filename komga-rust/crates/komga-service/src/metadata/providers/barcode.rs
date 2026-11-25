//! Kotlin `IsbnBarcodeProvider`: the ISBN of an EAN-13 barcode on the last or first pages.

use komga_types::{Book, validate_isbn};
use tracing::{debug, error};

use super::super::edit::patch::BookMetadataPatch;
use crate::reading::ReadingService;

const PAGES_LAST: usize = 3;
const PAGES_FIRST: usize = 3;

/// `getBookMetadataFromBook` for a ready book of `page_count` pages; `None` without a page
/// barcode that is a valid ISBN. Pages that cannot be read are logged and skipped.
pub(super) async fn book_patch(
    reading: &ReadingService,
    book: &Book,
    page_count: i32,
) -> Option<BookMetadataPatch> {
    for number in pages_to_try(page_count) {
        let found = reading
            .read_book_page_ean13(&book.id, number)
            .await
            .map_err(|error| error.to_string());
        let text = match found {
            Ok(Some(text)) => text,
            Ok(None) => {
                debug!(book = %book.id, page = number, "Book page does not contain a barcode");
                continue;
            }
            Err(message) => {
                error!(book = %book.id, page = number, "Error while processing page: {message}");
                continue;
            }
        };
        match validate_isbn(&text) {
            Some(isbn) => {
                debug!(book = %book.id, page = number, "Book page contains barcode which is valid ISBN: '{text}'");
                return Some(BookMetadataPatch {
                    isbn: Some(isbn),
                    ..BookMetadataPatch::default()
                });
            }
            None => {
                debug!(book = %book.id, page = number, "Book page contains barcode which is invalid ISBN: '{text}'");
            }
        }
    }
    None
}

/// The last pages from the end, then the first ones, each once.
fn pages_to_try(page_count: i32) -> Vec<i32> {
    let last = (1..=page_count).rev().take(PAGES_LAST);
    let first = (1..=page_count).take(PAGES_FIRST);
    let mut pages: Vec<i32> = Vec::new();
    for page in last.chain(first) {
        if !pages.contains(&page) {
            pages.push(page);
        }
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::pages_to_try;

    #[test]
    fn the_last_pages_are_tried_before_the_first_ones() {
        assert_eq!(pages_to_try(10), [10, 9, 8, 1, 2, 3]);
        assert_eq!(pages_to_try(4), [4, 3, 2, 1]);
        assert_eq!(pages_to_try(1), [1]);
    }
}
