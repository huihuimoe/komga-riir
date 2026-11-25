//! Kotlin `TransientBookDto`.

use komga_service::TransientBook;
use komga_types::{BookId, SeriesId};
use serde::Serialize;

use crate::mvc::media::format_size;
use crate::rest::dto::{JsonDateTime, PageDto, file_time_in_utc};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransientBookDto {
    id: BookId,
    name: String,
    /// The path of the file; only admins see transient books.
    url: String,
    file_last_modified: JsonDateTime,
    size_bytes: i64,
    size: String,
    status: &'static str,
    media_type: String,
    pages: Vec<PageDto>,
    files: Vec<String>,
    comment: String,
    number: Option<f32>,
    series_id: Option<SeriesId>,
}

impl From<TransientBook> for TransientBookDto {
    fn from(transient: TransientBook) -> Self {
        let pages = transient.pages().into_iter().enumerate();
        let TransientBook {
            book,
            media,
            number,
            series_id,
            ..
        } = transient;
        let url = book.url.to_path().expect("Books have file URLs");
        Self {
            id: book.id,
            name: book.name,
            url: url.to_string_lossy().into_owned(),
            file_last_modified: JsonDateTime(file_time_in_utc(book.file_last_modified)),
            size_bytes: book.file_size,
            size: format_size(book.file_size),
            status: media.status.as_str(),
            media_type: media.media_type.unwrap_or_default(),
            pages: pages
                .map(|(index, page)| PageDto::new(index, page))
                .collect(),
            files: media.files.into_iter().map(|file| file.file_name).collect(),
            comment: media.comment.unwrap_or_default(),
            number,
            series_id,
        }
    }
}
