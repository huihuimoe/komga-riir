//! Kotlin `EpubMetadataProvider`: metadata from the package document of an EPUB book.

use jiff::civil::Date;
use jiff::fmt::temporal::Pieces;
use komga_archive::epub::{EpubMetadata, EpubPackage};
use komga_archive::mobi::{self, MOBI_MEDIA_TYPE};
use komga_types::{
    Book, BookAuthor, Media, ReadingDirection, normalize_language_tag, validate_isbn,
};
use tokio::task;
use tracing::error;

use super::super::edit::patch::BookMetadataPatch;
use super::super::edit::series_patch::SeriesMetadataPatch;
use super::comic_rack::parse_float;

/// `getBookMetadataFromBook`; `None` without a readable package document.
pub(super) async fn book_patch(book: &Book, media: &Media) -> Option<BookMetadataPatch> {
    read(book, media).await.map(|metadata| patch(&metadata))
}

/// `getSeriesMetadataFromBook`; `None` without a readable package document.
pub(super) async fn series_patch(book: &Book, media: &Media) -> Option<SeriesMetadataPatch> {
    read(book, media).await.map(|metadata| series(&metadata))
}

/// Errors are logged.
async fn read(book: &Book, media: &Media) -> Option<EpubMetadata> {
    let Some(path) = book.url.to_path() else {
        error!(book = %book.id, url = %book.url, "Book URL is not a local file");
        return None;
    };
    let is_mobi = media.media_type.as_deref() == Some(MOBI_MEDIA_TYPE);
    let read = task::spawn_blocking(move || {
        if is_mobi {
            mobi::read_metadata(&path).map_err(|error| error.to_string())
        } else {
            EpubPackage::open(&path)
                .map(|package| package.metadata())
                .map_err(|error| error.to_string())
        }
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));
    read.inspect_err(|message| {
        error!(book = %book.id, "Error while getting metadata from EpubMetadataProvider: {message}");
    })
    .ok()
}

fn patch(metadata: &EpubMetadata) -> BookMetadataPatch {
    let authors: Vec<BookAuthor> = metadata
        .creators
        .iter()
        .map(|creator| BookAuthor::new(&creator.name, role(creator.role.as_deref())))
        .collect();
    let position = metadata.collection_position.as_deref();
    BookMetadataPatch {
        title: metadata.title.clone(),
        summary: metadata.description.clone(),
        number: position.map(str::to_owned),
        number_sort: position.and_then(parse_float),
        release_date: metadata.date.as_deref().and_then(release_date),
        authors: (!authors.is_empty()).then_some(authors),
        // Kotlin lowercases the identifier first, which fails ISBN-10s ending in `X`.
        isbn: metadata.identifiers.iter().find_map(|identifier| {
            let isbn = identifier
                .get(..5)
                .filter(|prefix| prefix.eq_ignore_ascii_case("isbn:"))
                .map_or(identifier.as_str(), |_| &identifier[5..]);
            validate_isbn(isbn)
        }),
        ..BookMetadataPatch::default()
    }
}

/// The MARC relator codes Komga knows; any other role is a writer.
fn role(code: Option<&str>) -> &'static str {
    match code {
        Some("clr") => "colorist",
        Some("cov") => "cover",
        Some("edt") => "editor",
        Some("art" | "ill") => "penciller",
        Some("trl") => "translator",
        _ => "writer",
    }
}

/// The date of an ISO 8601 date or date-time, as Kotlin's `ISO_DATE` and `ISO_DATE_TIME`
/// fallbacks read it; a year or a month alone is no date.
fn release_date(value: &str) -> Option<Date> {
    Pieces::parse(value).ok().map(|pieces| pieces.date())
}

fn series(metadata: &EpubMetadata) -> SeriesMetadataPatch {
    SeriesMetadataPatch {
        title: metadata.collection.clone(),
        title_sort: metadata.collection.clone(),
        reading_direction: match metadata.page_progression_direction.as_deref() {
            Some("rtl") => Some(ReadingDirection::RightToLeft),
            Some("ltr") => Some(ReadingDirection::LeftToRight),
            _ => None,
        },
        publisher: metadata.publisher.clone(),
        language: metadata
            .language
            .as_deref()
            .and_then(normalize_language_tag),
        genres: (!metadata.subjects.is_empty()).then(|| metadata.subjects.clone()),
        ..SeriesMetadataPatch::default()
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use super::release_date;

    #[test]
    fn release_dates_are_iso_dates_or_date_times() {
        for (value, expected) in [
            ("1999-07-31", Some(date(1999, 7, 31))),
            ("0101-01-01T00:00:00+00:00", Some(date(101, 1, 1))),
            ("2021-06-22T06:41:02Z", Some(date(2021, 6, 22))),
            ("2021-06-22T06:41", Some(date(2021, 6, 22))),
            ("2021", None),
            ("2021-06", None),
            ("2021-02-30", None),
            ("soon", None),
        ] {
            assert_eq!(release_date(value), expected, "{value}");
        }
    }
}
