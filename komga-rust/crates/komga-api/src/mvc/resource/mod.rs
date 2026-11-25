//! Spring MVC's handling of a `ResponseEntity<Resource>` answered with 200: `Accept-Ranges`, and
//! a `Range` request served as 206 (`HttpRange`, `ResourceRegionHttpMessageConverter`) or 416.

use std::io::SeekFrom;

use axum::body::{Body, Bytes};
use axum::http::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::fs::File;
use tokio::io::{self, AsyncReadExt as _, AsyncSeekExt as _, Take};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;

/// Each read is a trip to the blocking pool or io-uring; with `ReaderStream`'s 4 KiB default a
/// 1 GiB download took about 7 times longer.
pub(crate) const STREAM_CHUNK: usize = 64 * 1024;
/// Bounded in-flight chunk queue for streaming multipart byte-range responses.
const MULTIPART_CHANNEL_CAPACITY: usize = 4;

const MAX_RANGES: usize = 100;
const BOUNDARY_CHARS: &[u8] = b"-_1234567890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

pub(crate) enum Resource {
    Memory(Bytes),
    /// A file cut at `length` so that the body never exceeds the announced length.
    File {
        content: Take<File>,
        length: u64,
    },
}

impl Resource {
    fn length(&self) -> u64 {
        match self {
            Self::Memory(bytes) => bytes.len() as u64,
            Self::File { length, .. } => *length,
        }
    }

    fn into_body(self) -> Body {
        match self {
            Self::Memory(bytes) => Body::from(bytes),
            Self::File { content, .. } => {
                Body::from_stream(ReaderStream::with_capacity(content, STREAM_CHUNK))
            }
        }
    }

    async fn into_region_body(self, region: Region) -> io::Result<Body> {
        match self {
            Self::Memory(bytes) => Ok(Body::from(slice_memory(&bytes, region)?)),
            Self::File { mut content, .. } => {
                content
                    .get_mut()
                    .seek(SeekFrom::Start(region.start))
                    .await?;
                content.set_limit(region.count);
                Ok(Body::from_stream(ReaderStream::with_capacity(
                    content,
                    STREAM_CHUNK,
                )))
            }
        }
    }
}

fn slice_memory(bytes: &Bytes, region: Region) -> io::Result<Bytes> {
    let start = usize::try_from(region.start)
        .ok()
        .filter(|start| *start <= bytes.len());
    let end = region
        .start
        .checked_add(region.count)
        .and_then(|end| usize::try_from(end).ok())
        .filter(|end| *end <= bytes.len());
    match (start, end) {
        (Some(start), Some(end)) => Ok(bytes.slice(start..end)),
        _ => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Range exceeds memory buffer",
        )),
    }
}

#[derive(Clone, Copy)]
struct Region {
    start: u64,
    count: u64,
}

impl Region {
    fn end(self) -> u64 {
        self.start + self.count - 1
    }
}

/// The response to `request` for `resource`, whose `headers` hold its type and the controller's
/// headers.
pub(crate) async fn response(
    request: &HeaderMap,
    mut headers: HeaderMap,
    resource: Resource,
) -> io::Result<Response> {
    headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let length = resource.length();
    let Some(range) = request.get(RANGE) else {
        return Ok(whole(StatusCode::OK, headers, resource));
    };
    let Some(regions) = regions(range.as_bytes(), length) else {
        headers.insert(CONTENT_RANGE, ascii(format!("bytes */{length}")));
        return Ok(whole(StatusCode::RANGE_NOT_SATISFIABLE, headers, resource));
    };
    if let [region] = regions[..] {
        let content_range = format!("bytes {}-{}/{length}", region.start, region.end());
        headers.insert(CONTENT_RANGE, ascii(content_range));
        headers.insert(CONTENT_LENGTH, HeaderValue::from(region.count));
        let body = resource.into_region_body(region).await?;
        return Ok((StatusCode::PARTIAL_CONTENT, headers, body).into_response());
    }

    let boundary = boundary();
    let part_type = headers.get(CONTENT_TYPE).cloned();
    let total_length = multipart_length(&regions, length, &boundary, part_type.as_ref());
    if let Some(total_length) = total_length {
        headers.insert(CONTENT_LENGTH, HeaderValue::from(total_length));
    }
    let multipart = format!("multipart/byteranges; boundary={boundary}");
    headers.insert(CONTENT_TYPE, ascii(multipart));
    let body = stream_multipart(resource, regions, length, boundary, part_type);
    Ok((StatusCode::PARTIAL_CONTENT, headers, body).into_response())
}

fn multipart_length(
    regions: &[Region],
    length: u64,
    boundary: &str,
    part_type: Option<&HeaderValue>,
) -> Option<u64> {
    let mut total: u64 = 0;
    for region in regions {
        let header = part_header(boundary, part_type, *region, length);
        total = total.checked_add(header.len() as u64)?;
        total = total.checked_add(region.count)?;
    }
    total.checked_add((boundary.len() + 6) as u64)
}

fn part_header(
    boundary: &str,
    part_type: Option<&HeaderValue>,
    region: Region,
    length: u64,
) -> Vec<u8> {
    let mut header = Vec::new();
    header.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    if let Some(part_type) = part_type {
        header.extend_from_slice(b"Content-Type: ");
        header.extend_from_slice(part_type.as_bytes());
        header.extend_from_slice(b"\r\n");
    }
    header.extend_from_slice(
        format!(
            "Content-Range: bytes {}-{}/{length}\r\n\r\n",
            region.start,
            region.end()
        )
        .as_bytes(),
    );
    header
}

fn stream_multipart(
    resource: Resource,
    regions: Vec<Region>,
    length: u64,
    boundary: String,
    part_type: Option<HeaderValue>,
) -> Body {
    let (tx, rx) = mpsc::channel::<io::Result<Bytes>>(MULTIPART_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        if let Err(error) = write_multipart(
            &tx,
            resource,
            regions,
            length,
            &boundary,
            part_type.as_ref(),
        )
        .await
        {
            let _ = tx.send(Err(error)).await;
        }
    });
    Body::from_stream(ReceiverStream::new(rx))
}

async fn write_multipart(
    tx: &mpsc::Sender<io::Result<Bytes>>,
    mut resource: Resource,
    regions: Vec<Region>,
    length: u64,
    boundary: &str,
    part_type: Option<&HeaderValue>,
) -> io::Result<()> {
    for region in regions {
        let header = Bytes::from(part_header(boundary, part_type, region, length));
        if tx.send(Ok(header)).await.is_err() {
            return Ok(());
        }
        match &mut resource {
            Resource::Memory(bytes) => {
                if tx.send(Ok(slice_memory(bytes, region)?)).await.is_err() {
                    return Ok(());
                }
            }
            Resource::File { content, .. } => {
                let file = content.get_mut();
                file.seek(SeekFrom::Start(region.start)).await?;
                let mut remaining = region.count;
                while remaining > 0 {
                    let to_read = usize::try_from(remaining.min(STREAM_CHUNK as u64))
                        .expect("Chunk fits in usize");
                    let mut buf = vec![0_u8; to_read];
                    file.read_exact(&mut buf).await?;
                    remaining -= to_read as u64;
                    if tx.send(Ok(Bytes::from(buf))).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }
    let _ = tx
        .send(Ok(Bytes::from(format!("\r\n--{boundary}--"))))
        .await;
    Ok(())
}

fn ascii(value: String) -> HeaderValue {
    HeaderValue::try_from(value).expect("Ranges and boundaries are ASCII")
}

fn whole(status: StatusCode, mut headers: HeaderMap, resource: Resource) -> Response {
    headers.insert(CONTENT_LENGTH, HeaderValue::from(resource.length()));
    (status, headers, resource.into_body()).into_response()
}

/// `HttpRange.parseRanges` then `toResourceRegions`; `None` where Spring throws. An empty header
/// or one without ranges asks for no region at all, which Spring answers with an empty multipart.
fn regions(header: &[u8], length: u64) -> Option<Vec<Region>> {
    // Bytes beyond ASCII never parse as a range.
    let header = str::from_utf8(header).ok()?;
    if header.is_empty() {
        return Some(Vec::new());
    }
    // `StringUtils.tokenizeToStringArray`: trimmed, without empty tokens.
    let tokens: Vec<&str> = header
        .strip_prefix("bytes=")?
        .split(',')
        .map(|token| token.trim_matches(|ch: char| ch <= ' '))
        .filter(|token| !token.is_empty())
        .collect();
    if tokens.len() > MAX_RANGES {
        return None;
    }
    let regions = tokens
        .iter()
        .map(|token| region(token, length))
        .collect::<Option<Vec<Region>>>()?;
    let total: u64 = regions.iter().map(|region| region.count).sum();
    (regions.len() < 2 || total < length).then_some(regions)
}

/// One `bytes=` token resolved against `length`, which Spring requires to be positive.
fn region(token: &str, length: u64) -> Option<Region> {
    // Java `Long.parseLong` takes an optional sign, as `i64::from_str` does.
    let length = i64::try_from(length).ok().filter(|length| *length > 0)?;
    let (first, last) = token.split_once('-')?;
    let (start, end) = if first.is_empty() {
        let suffix: i64 = last.parse().ok().filter(|suffix| *suffix >= 0)?;
        ((length - suffix).max(0), length - 1)
    } else {
        let first: i64 = first.parse().ok().filter(|first| *first >= 0)?;
        let last: Option<i64> = match last {
            "" => None,
            last => Some(last.parse().ok().filter(|last| *last >= first)?),
        };
        (
            first,
            last.filter(|last| *last < length).unwrap_or(length - 1),
        )
    };
    (start < length).then(|| Region {
        start: start.unsigned_abs(),
        count: (end - start + 1).unsigned_abs(),
    })
}

/// `MimeTypeUtils.generateMultipartBoundaryString`: 30 to 40 random boundary characters.
fn boundary() -> String {
    let mut random = [0_u8; 41];
    getrandom::fill(&mut random).expect("Failed to get random bytes");
    let length = 30 + usize::from(random[0] % 11);
    random[1..=length]
        .iter()
        .map(|byte| char::from(BOUNDARY_CHARS[usize::from(*byte) % BOUNDARY_CHARS.len()]))
        .collect()
}

#[cfg(test)]
mod tests;
