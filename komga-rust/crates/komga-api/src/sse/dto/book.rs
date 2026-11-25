use komga_types::{BookId, BookImport, BookRef, LibraryId, SeriesId};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct BookSseDto<'a> {
    pub book_id: &'a BookId,
    pub series_id: &'a SeriesId,
    pub library_id: &'a LibraryId,
}

impl<'a> From<&'a BookRef> for BookSseDto<'a> {
    fn from(book: &'a BookRef) -> Self {
        Self {
            book_id: &book.book_id,
            series_id: &book.series_id,
            library_id: &book.library_id,
        }
    }
}

/// Kotlin `BookImportSseDto`: `message` is null on success.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct BookImportSseDto<'a> {
    pub book_id: Option<&'a BookId>,
    pub source_file: &'a str,
    pub success: bool,
    pub message: Option<&'a str>,
}

impl<'a> From<&'a BookImport> for BookImportSseDto<'a> {
    fn from(import: &'a BookImport) -> Self {
        Self {
            book_id: import.book_id.as_ref(),
            source_file: &import.source_file,
            success: import.book_id.is_some(),
            message: import.message.as_deref(),
        }
    }
}
