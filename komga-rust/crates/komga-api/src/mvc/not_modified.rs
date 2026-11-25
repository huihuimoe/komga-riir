//! Spring `ServletWebRequest.checkNotModified(lastModified)` for GET and HEAD without an ETag,
//! and Kotlin `setNotModified`'s response headers.

use std::time::{Duration, SystemTime};

use axum::http::header::{CACHE_CONTROL, IF_MODIFIED_SINCE, IF_UNMODIFIED_SINCE, LAST_MODIFIED};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use super::http_cache::CACHE_PRIVATE;

/// Whether the request's validators let Spring answer 304, comparing whole seconds.
/// `If-Unmodified-Since` wins over `If-Modified-Since`; a GET whose resource changed after it
/// also gets 304, because the controller answers every positive check with 304.
pub(crate) fn check(headers: &HeaderMap, last_modified: i64) -> bool {
    if last_modified < 0 {
        return false;
    }
    let last_modified = last_modified / 1000 * 1000;
    if let Some(since) = date_header(headers, &IF_UNMODIFIED_SINCE) {
        return since < last_modified;
    }
    date_header(headers, &IF_MODIFIED_SINCE).is_some_and(|since| since >= last_modified)
}

/// The 304 response of the page controllers.
pub(crate) fn response(last_modified: i64) -> Response {
    (StatusCode::NOT_MODIFIED, cache_headers(last_modified)).into_response()
}

/// `setNotModified(media)`: private caching that revalidates, and `Last-Modified` when the
/// timestamp is at or after the Unix epoch (pre-1970 mtimes omit `Last-Modified`).
pub(crate) fn cache_headers(last_modified: i64) -> HeaderMap {
    let mut headers = HeaderMap::with_capacity(2);
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(CACHE_PRIVATE));
    if let Ok(millis) = u64::try_from(last_modified) {
        let time = SystemTime::UNIX_EPOCH + Duration::from_millis(millis);
        let date =
            HeaderValue::from_str(&httpdate::fmt_http_date(time)).expect("HTTP dates are ASCII");
        headers.insert(LAST_MODIFIED, date);
    }
    headers
}

/// Spring `parseDateHeader`: the first value as an HTTP date in epoch milliseconds, also
/// accepting IE 10's `<date>; length=<n>`; `None` when absent or not a date.
fn date_header(headers: &HeaderMap, name: &HeaderName) -> Option<i64> {
    let value = headers.get(name)?.to_str().ok()?;
    let time = httpdate::parse_http_date(value)
        .or_else(|error| match value.split_once(';') {
            Some((date, _)) => httpdate::parse_http_date(date),
            None => Err(error),
        })
        .ok()?;
    let since_epoch = time.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_millis()).ok()
}

#[cfg(test)]
mod tests {
    use axum::http::header::{
        CACHE_CONTROL, IF_MODIFIED_SINCE, IF_UNMODIFIED_SINCE, LAST_MODIFIED,
    };
    use axum::http::{HeaderMap, HeaderValue};

    use super::{CACHE_PRIVATE, cache_headers, check};

    /// 1994-11-06T08:49:37.250Z: dates compare whole seconds.
    const LAST_MODIFIED_MS: i64 = 784_111_777_250;
    const AT: &str = "Sun, 06 Nov 1994 08:49:37 GMT";
    const BEFORE: &str = "Sun, 06 Nov 1994 08:49:36 GMT";

    fn headers(pairs: &[(&axum::http::HeaderName, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(*name, HeaderValue::from_str(value).expect("Header value"));
        }
        headers
    }

    #[test]
    fn validators_compare_whole_seconds_in_spring_order() {
        for (pairs, expected) in [
            (vec![], false),
            (vec![(&IF_MODIFIED_SINCE, AT)], true),
            (vec![(&IF_MODIFIED_SINCE, BEFORE)], false),
            (
                vec![(&IF_MODIFIED_SINCE, "Sunday, 06-Nov-94 08:49:37 GMT")],
                true,
            ),
            (vec![(&IF_MODIFIED_SINCE, "Sun Nov  6 08:49:37 1994")], true),
            (
                vec![(
                    &IF_MODIFIED_SINCE,
                    "Sun, 06 Nov 1994 08:49:37 GMT; length=13",
                )],
                true,
            ),
            (vec![(&IF_MODIFIED_SINCE, "yesterday")], false),
            (vec![(&IF_UNMODIFIED_SINCE, BEFORE)], true),
            (vec![(&IF_UNMODIFIED_SINCE, AT)], false),
            // A valid If-Unmodified-Since decides alone; an invalid one is ignored.
            (
                vec![(&IF_UNMODIFIED_SINCE, AT), (&IF_MODIFIED_SINCE, AT)],
                false,
            ),
            (
                vec![(&IF_UNMODIFIED_SINCE, "never"), (&IF_MODIFIED_SINCE, AT)],
                true,
            ),
        ] {
            assert_eq!(
                check(&headers(&pairs), LAST_MODIFIED_MS),
                expected,
                "{pairs:?}"
            );
        }
    }

    #[test]
    fn pre_epoch_last_modified_omits_last_modified_header_without_panicking() {
        let pre_epoch = cache_headers(-1_000);
        assert_eq!(
            pre_epoch.get(CACHE_CONTROL),
            Some(&HeaderValue::from_static(CACHE_PRIVATE))
        );
        assert!(pre_epoch.get(LAST_MODIFIED).is_none());

        let epoch = cache_headers(0);
        assert!(epoch.get(LAST_MODIFIED).is_some());
    }
}
