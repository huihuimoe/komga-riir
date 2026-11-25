//! Shared HTTP media, page streaming, and byte-size formatting helpers across REST, OPDS, and Kobo.

use axum::body::Body;
use axum::http::header::{
    CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE,
};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use komga_service::{BookArchive, BookFile, PageContent, ReadingService};
use komga_types::{BookId, KomgaUser};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;

use crate::accept::{self, MediaRange};
use crate::error::ApiError;
use crate::mvc::resource::STREAM_CHUNK;
use crate::mvc::{content_disposition, not_modified};

/// A JPEG image body with `Content-Type: image/jpeg`.
pub(crate) fn jpeg(bytes: Vec<u8>) -> Response {
    (
        [(CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))],
        bytes,
    )
        .into_response()
}

/// A `StreamingResponseBody` ZIP download: sent as it is written, without a length.
pub(crate) fn book_archive_response(archive: BookArchive) -> Response {
    let disposition = content_disposition::header("attachment", &archive.file_name);
    (
        [
            (CONTENT_TYPE, HeaderValue::from_static("application/zip")),
            (CONTENT_DISPOSITION, disposition),
        ],
        Body::from_stream(ReceiverStream::new(archive.chunks)),
    )
        .into_response()
}

/// A book file as an attachment, streamed.
pub(crate) fn file_response(file: BookFile) -> Response {
    let disposition = content_disposition::header("attachment", &file.file_name);
    let body = Body::from_stream(ReaderStream::with_capacity(file.content, STREAM_CHUNK));
    (
        [
            (
                CONTENT_TYPE,
                media_type_or_default(file.media_type.as_deref()),
            ),
            (CONTENT_LENGTH, HeaderValue::from(file.length)),
            (CONTENT_DISPOSITION, disposition),
        ],
        body,
    )
        .into_response()
}

/// Parameters identifying the book page and format requested by a client.
pub(crate) struct PageQuery<'a> {
    pub(crate) id: &'a BookId,
    pub(crate) number: i32,
    pub(crate) convert: Option<&'a str>,
    pub(crate) accept: Option<Vec<MediaRange>>,
}

/// Kotlin `getBookPageInternal`: a PDF page is served raw when `accept` prefers PDF.
pub(crate) async fn page_internal(
    user: &KomgaUser,
    reading: &ReadingService,
    query: PageQuery<'_>,
    headers: &HeaderMap,
    path: &str,
) -> Result<Response, ApiError> {
    let PageQuery {
        id,
        number,
        convert,
        accept,
    } = query;
    let reading_error = |error| ApiError::from_reading(&error, path);
    let source = reading.page_source(user, id).await.map_err(reading_error)?;
    let last_modified = source.last_modified();
    if not_modified::check(headers, last_modified) {
        return Ok(not_modified::response(last_modified));
    }
    let name = format!("{}-{number}", source.book_name());
    if source.is_pdf() && accept.is_some_and(accept::prefers_pdf) {
        let content = reading
            .read_raw_page(user, source, number)
            .await
            .map_err(reading_error)?;
        return Ok(page_response(&name, "", content, last_modified));
    }
    let content = reading
        .read_page(user, source, number, convert)
        .await
        .map_err(reading_error)?;
    Ok(page_response(&name, "jpeg", content, last_modified))
}

/// The `Accept` ranges of a request the `produces` condition let through.
pub(crate) fn media_ranges(headers: &HeaderMap) -> Vec<MediaRange> {
    accept::parse(headers).expect("The produces condition rejects an unparseable Accept")
}

/// Kotlin `getMediaTypeOrDefault`: EPUB files carry the media types their package declares.
pub(crate) fn media_type_or_default(media_type: Option<&str>) -> HeaderValue {
    media_type
        .filter(|media_type| accept::is_media_type(media_type))
        .and_then(|media_type| HeaderValue::from_str(media_type).ok())
        .unwrap_or(HeaderValue::from_static("application/octet-stream"))
}

/// The page inline, named `<book>-<number><extension>`; Kotlin falls back to `default_extension`
/// (without a dot) when the media type has no known extension.
pub(crate) fn page_response(
    name: &str,
    default_extension: &str,
    content: PageContent,
    last_modified: i64,
) -> Response {
    let extension = content.extension.unwrap_or(default_extension);
    let content_type = media_type_or_default(Some(&content.media_type));
    let disposition = content_disposition::header("inline", &format!("{name}{extension}"));
    (
        not_modified::cache_headers(last_modified),
        [
            (CONTENT_TYPE, content_type),
            (CONTENT_DISPOSITION, disposition),
            // Pages can be SVG or HTML-like content opened directly in the browser.
            (
                CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
            ),
        ],
        content.bytes,
    )
        .into_response()
}

/// byteunits `BinaryByteUnit.format`: `#,##0.#` (half-even) in 1024 steps up to PiB. Java formats
/// with the server's default locale; this is the English form (`1,023.5 KiB`).
pub(crate) fn format_size(bytes: i64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    // Java converts the long to a double too.
    let mut count = bytes as f64;
    let mut unit = 0;
    while count >= 1024.0 && unit < UNITS.len() - 1 {
        count /= 1024.0;
        unit += 1;
    }
    // Rust rounds the exact binary value half to even, as `DecimalFormat` does.
    let rounded = format!("{count:.1}");
    let (integer, fraction) = rounded.split_once('.').expect("One decimal");
    let mut grouped = String::new();
    for (position, digit) in integer.chars().enumerate() {
        if position > 0 && (integer.len() - position) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if fraction != "0" {
        grouped.push('.');
        grouped.push_str(fraction);
    }
    format!("{grouped} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::format_size;

    #[test]
    fn sizes_format_like_byteunits() {
        // byteunits 0.9.1 output in an English locale.
        for (bytes, expected) in [
            (0, "0 B"),
            (999, "999 B"),
            (1000, "1,000 B"),
            (1023, "1,023 B"),
            (1024, "1 KiB"),
            (1075, "1 KiB"),
            (1126, "1.1 KiB"),
            (1177, "1.1 KiB"),
            (1280, "1.2 KiB"),
            (1331, "1.3 KiB"),
            (1587, "1.5 KiB"),
            (1638, "1.6 KiB"),
            (2662, "2.6 KiB"),
            (1_048_575, "1,024 KiB"),
            (1_048_576, "1 MiB"),
            (5_000_000, "4.8 MiB"),
            (123_456_789, "117.7 MiB"),
            (1_649_267_441_664_000, "1.5 PiB"),
            (2_251_799_813_685_248_000, "2,000 PiB"),
            (i64::MAX, "8,192 PiB"),
        ] {
            assert_eq!(format_size(bytes), expected, "{bytes}");
        }
    }
}
