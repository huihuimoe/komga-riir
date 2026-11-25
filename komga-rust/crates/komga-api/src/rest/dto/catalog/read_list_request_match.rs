//! Kotlin `ReadListRequestMatchDto`.

use jiff::civil::Date;
use komga_service::{ReadListMatch, ReadListRequestMatch};
use komga_types::{BookId, ReadListBookMatch, ReadListSeriesMatch, SeriesId};
use serde::Serialize;

/// Kotlin `ReadListMatch.errorCode` for a name another read list holds.
const DUPLICATE_NAME: &str = "ERR_1009";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadListRequestMatchDto {
    read_list_match: ReadListMatchDto,
    requests: Vec<ReadListRequestBookMatchesDto>,
    /// Kotlin never sets it.
    error_code: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadListMatchDto {
    name: String,
    error_code: &'static str,
}

#[derive(Debug, Serialize)]
struct ReadListRequestBookMatchesDto {
    request: ReadListRequestBookDto,
    matches: Vec<ReadListRequestBookMatchDto>,
}

#[derive(Debug, Serialize)]
struct ReadListRequestBookDto {
    series: Vec<String>,
    number: String,
}

#[derive(Debug, Serialize)]
struct ReadListRequestBookMatchDto {
    series: ReadListRequestBookMatchSeriesDto,
    books: Vec<ReadListRequestBookMatchBookDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadListRequestBookMatchSeriesDto {
    series_id: SeriesId,
    title: String,
    /// `yyyy-MM-dd`.
    release_date: Option<Date>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadListRequestBookMatchBookDto {
    book_id: BookId,
    number: String,
    title: String,
}

impl From<ReadListRequestMatch> for ReadListRequestMatchDto {
    fn from(found: ReadListRequestMatch) -> Self {
        Self {
            read_list_match: ReadListMatchDto {
                name: found.name,
                error_code: if found.duplicate_name {
                    DUPLICATE_NAME
                } else {
                    ""
                },
            },
            requests: found.requests.into_iter().map(request_matches).collect(),
            error_code: "",
        }
    }
}

fn request_matches(found: ReadListMatch) -> ReadListRequestBookMatchesDto {
    ReadListRequestBookMatchesDto {
        request: ReadListRequestBookDto {
            series: found.request.series,
            number: found.request.number,
        },
        matches: found.matches.into_iter().map(series_match).collect(),
    }
}

fn series_match(found: ReadListSeriesMatch) -> ReadListRequestBookMatchDto {
    ReadListRequestBookMatchDto {
        series: ReadListRequestBookMatchSeriesDto {
            series_id: found.series_id,
            title: found.series_title,
            release_date: found.release_date,
        },
        books: found.books.into_iter().map(book_match).collect(),
    }
}

fn book_match(found: ReadListBookMatch) -> ReadListRequestBookMatchBookDto {
    ReadListRequestBookMatchBookDto {
        book_id: found.book_id,
        number: found.number,
        title: found.title,
    }
}
