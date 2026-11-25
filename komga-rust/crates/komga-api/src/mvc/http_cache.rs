//! Kotlin's HTTP caching around the handlers: `WebContentInterceptor` with `cachePrivate` for
//! `/api/**` and `/opds/**` (`WebMvcConfiguration`), and `ShallowEtagHeaderFilter` for `/api/*`,
//! `/opds/*` and `/kobo/*` except file downloads (`EtagFilterConfiguration`).

use std::convert::Infallible;

use axum::body::{Body, Bytes, HttpBody as _};
use axum::extract::Request;
use axum::http::header::{CACHE_CONTROL, CONTENT_LENGTH, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use md5::{Digest, Md5};
use tokio::task;
use tokio_stream::StreamExt as _;

use crate::error::ApiError;

/// Kotlin `cachePrivate`: `CacheControl.maxAge(0).cachePrivate().mustRevalidate()`.
pub(crate) const CACHE_PRIVATE: &str = "max-age=0, must-revalidate, private";

/// Maximum response body size buffered for shallow ETag hashing; larger responses stream through
/// untouched without an ETag to prevent unbounded memory growth.
const MAX_ETAG_BODY_BYTES: usize = 16 * 1024 * 1024;
/// Bodies above this size are hashed on Tokio's blocking pool to keep async workers responsive.
const BLOCKING_HASH_THRESHOLD: usize = 64 * 1024;

/// Streamed downloads, which the filter must not buffer. Kotlin leaves out the OPDS v2 one and
/// buffers whole books there.
const ETAG_EXCLUDED: [&str; 6] = [
    "/api/v1/books/*/file/**",
    "/opds/v1.2/books/*/file/**",
    "/opds/v2/books/*/file/**",
    "/api/v1/readlists/*/file/**",
    "/api/v1/series/*/file/**",
    "/kobo/*/v1/books/*/file/**",
];

/// Marks a response that Spring Security writes before any handler runs, so that neither the
/// interceptor nor the ETag filter, which sit behind it, see it.
#[derive(Clone, Copy)]
struct BeforeHandler;

pub(crate) fn before_handler(mut response: Response) -> Response {
    response.extensions_mut().insert(BeforeHandler);
    response
}

/// The caching layer around every route.
pub async fn layer(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let cache_private = ["/api", "/opds"].iter().any(|root| under(path, root));
    let etag = request.method() == Method::GET
        && ["/api", "/opds", "/kobo"]
            .iter()
            .any(|root| under(path, root))
        && !ETAG_EXCLUDED.iter().any(|pattern| matches(pattern, path));
    let etag_context = etag.then(|| {
        let if_none_match: Vec<HeaderValue> = request
            .headers()
            .get_all(IF_NONE_MATCH)
            .iter()
            .cloned()
            .collect();
        (request.uri().clone(), if_none_match)
    });

    let mut response = next.run(request).await;
    if response.extensions().get::<BeforeHandler>().is_some() {
        return response;
    }
    // The interceptor sets the header first; a handler's own value replaces it.
    if cache_private && !response.headers().contains_key(CACHE_CONTROL) {
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static(CACHE_PRIVATE));
    }
    let no_store = response
        .headers()
        .get(CACHE_CONTROL)
        .is_some_and(|value| String::from_utf8_lossy(value.as_bytes()).contains("no-store"));
    let Some((uri, if_none_match)) = etag_context else {
        return response;
    };
    if !response.status().is_success() || no_store || response.headers().contains_key(ETAG) {
        return response;
    }
    shallow_etag(response, &if_none_match, uri.path()).await
}

/// `ShallowEtagHeaderFilter.updateResponse`: a strong `"0<MD5 of the body>"`, and 304 without
/// a body when a 200 matches `If-None-Match` (weak comparison). Responses larger than
/// [`MAX_ETAG_BODY_BYTES`] bypass ETag buffering and stream through directly.
async fn shallow_etag(response: Response, if_none_match: &[HeaderValue], path: &str) -> Response {
    let (mut parts, body) = response.into_parts();
    if body.size_hint().lower() > MAX_ETAG_BODY_BYTES as u64 {
        return Response::from_parts(parts, body);
    }
    let mut stream = body.into_data_stream();
    let mut chunks: Vec<Bytes> = Vec::new();
    let mut total: usize = 0;
    while let Some(next) = stream.next().await {
        let chunk = match next {
            Ok(chunk) => chunk,
            Err(error) => return ApiError::internal_error(path, error).into_response(),
        };
        total = total.saturating_add(chunk.len());
        chunks.push(chunk);
        if total > MAX_ETAG_BODY_BYTES {
            let prefix = tokio_stream::iter(chunks.into_iter().map(Ok));
            return Response::from_parts(parts, Body::from_stream(prefix.chain(stream)));
        }
    }
    let (digest, chunks) = match hash_body(chunks, total).await {
        Ok(result) => result,
        Err(error) => return ApiError::internal_error(path, error).into_response(),
    };
    let tag = format!("0{}", hex::encode(digest));
    let header = HeaderValue::try_from(format!("\"{tag}\"")).expect("Hex is ASCII");
    parts.headers.insert(ETAG, header);
    let matched = if_none_match.iter().any(|value| {
        let value = String::from_utf8_lossy(value.as_bytes());
        requested_tags(&value).contains(&tag.as_str())
    });
    if parts.status == StatusCode::OK && matched {
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(CONTENT_LENGTH);
        return Response::from_parts(parts, Body::empty());
    }
    let body = match chunks.len() {
        0 => Body::empty(),
        1 => Body::from(chunks.into_iter().next().expect("Checked length")),
        _ => {
            parts
                .headers
                .entry(CONTENT_LENGTH)
                .or_insert_with(|| HeaderValue::from(total));
            Body::from_stream(tokio_stream::iter(
                chunks.into_iter().map(Ok::<Bytes, Infallible>),
            ))
        }
    };
    Response::from_parts(parts, body)
}

async fn hash_body(
    chunks: Vec<Bytes>,
    total: usize,
) -> Result<([u8; 16], Vec<Bytes>), task::JoinError> {
    if total > BLOCKING_HASH_THRESHOLD {
        task::spawn_blocking(move || (hash_chunks(&chunks), chunks)).await
    } else {
        Ok((hash_chunks(&chunks), chunks))
    }
}

fn hash_chunks(chunks: &[Bytes]) -> [u8; 16] {
    let mut hasher = Md5::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    hasher.finalize().into()
}

/// Ant `<root>/**` and servlet `<root>/*`: the root itself or anything below it.
pub(crate) fn under(path: &str, root: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// A `PathPattern` of literal segments, `*` for one segment and a trailing `/**` for any rest.
fn matches(pattern: &str, path: &str) -> bool {
    let mut segments = path.split('/');
    for expected in pattern.split('/') {
        if expected == "**" {
            return true;
        }
        match segments.next() {
            Some(segment) if expected == "*" || expected == segment => {}
            _ => return false,
        }
    }
    segments.next().is_none()
}

/// The values of the quoted tags in an `If-None-Match` value (Spring `ETag.parse`). A `*` or an
/// unquoted token makes the parser skip to the next comma; an unclosed tag is dropped.
fn requested_tags(value: &str) -> Vec<&str> {
    enum Quotes {
        Before,
        Inside(usize),
        After,
    }
    let mut tags = Vec::new();
    let mut state = Quotes::Before;
    let mut index = 0;
    while let Some(&byte) = value.as_bytes().get(index) {
        match state {
            Quotes::Inside(start) => {
                if byte == b'"' {
                    let tag = &value[start..index];
                    if !tag.trim().is_empty() {
                        tags.push(tag);
                    }
                    state = Quotes::After;
                }
            }
            _ if byte.is_ascii_whitespace() => {}
            _ if byte == b',' => state = Quotes::Before,
            Quotes::Before if byte == b'*' => state = Quotes::After,
            Quotes::Before if byte == b'"' => state = Quotes::Inside(index + 1),
            Quotes::Before if value[index..].starts_with("W/\"") => {
                state = Quotes::Inside(index + 3);
                index += 2;
            }
            Quotes::Before | Quotes::After => {}
        }
        index += 1;
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::{matches, requested_tags, under};

    #[test]
    fn if_none_match_parses_like_spring_etag() {
        for (value, expected) in [
            ("\"a\"", vec!["a"]),
            ("W/\"a\", \"b\"", vec!["a", "b"]),
            ("\"a\"\"b\"", vec!["a"]),
            ("* , \"a\"", vec!["a"]),
            ("* \"a\"", vec![]),
            ("abc, \"a\"", vec!["a"]),
            // Not a weak prefix: `w` and `/` are skipped, then the tag is read.
            ("w/\"a\"", vec!["a"]),
            ("\" \", \"a b\"", vec!["a b"]),
            ("\"a", vec![]),
        ] {
            assert_eq!(requested_tags(value), expected, "{value:?}");
        }
    }

    #[test]
    fn paths_match_like_servlet_and_path_patterns() {
        assert!(under("/api", "/api"));
        assert!(under("/api/v1/books", "/api"));
        assert!(!under("/apis", "/api"));
        let pattern = "/api/v1/books/*/file/**";
        assert!(matches(pattern, "/api/v1/books/1/file"));
        assert!(matches(pattern, "/api/v1/books/1/file/name.cbz"));
        assert!(!matches(pattern, "/api/v1/books/1/files"));
        assert!(!matches(pattern, "/api/v1/books/1/2/file"));
        assert!(matches(
            "/kobo/*/v1/books/*/file/**",
            "/kobo/t/v1/books/1/file/epub"
        ));
    }
}
