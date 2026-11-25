use axum::body::{self, Bytes};
use axum::http::header::RANGE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};

use super::{Resource, regions, response};

fn resolved(header: &str, length: u64) -> Option<Vec<(u64, u64)>> {
    regions(header.as_bytes(), length).map(|regions| {
        regions
            .iter()
            .map(|region| (region.start, region.count))
            .collect()
    })
}

#[test]
fn ranges_resolve_like_spring_http_range() {
    for (header, expected) in [
        ("bytes=0-3", Some(vec![(0, 4)])),
        ("bytes=6-", Some(vec![(6, 4)])),
        ("bytes=8-100", Some(vec![(8, 2)])),
        ("bytes=-3", Some(vec![(7, 3)])),
        ("bytes=-30", Some(vec![(0, 10)])),
        ("bytes=+1-+2", Some(vec![(1, 2)])),
        ("bytes= 0-1 ,, 4-5 ,", Some(vec![(0, 2), (4, 2)])),
        ("bytes=5-6,0-1", Some(vec![(5, 2), (0, 2)])),
        ("", Some(vec![])),
        ("bytes=", Some(vec![])),
        ("bytes=0-4,5-9", None),
        ("bytes=0-0,0-0,0-0,0-0,0-0,0-0,0-0,0-0,0-0,0-0", None),
        ("bytes=10-", None),
        ("bytes=-0", None),
        ("bytes=3-2", None),
        ("bytes=1--3", None),
        ("bytes=--3", None),
        ("bytes=a-3", None),
        ("bytes=3", None),
        ("Bytes=0-1", None),
        ("items=0-1", None),
    ] {
        assert_eq!(resolved(header, 10), expected, "{header:?}");
    }
    assert_eq!(
        resolved("bytes=0-", 0),
        None,
        "Spring requires a positive length"
    );
    assert_eq!(resolved("bytes=", 0), Some(vec![]));
    let many = format!("bytes={}", ["0-0"; 101].join(","));
    assert_eq!(resolved(&many, 1000), None, "More than 100 ranges");
    assert!(regions(b"bytes=0-\xff", 10).is_none());
}

#[tokio::test]
async fn multi_range_streams_without_content_type_header() {
    let mut request = HeaderMap::new();
    request.insert(RANGE, HeaderValue::from_static("bytes=0-1, 4-5"));
    let response = response(
        &request,
        HeaderMap::new(),
        Resource::Memory(Bytes::from_static(b"0123456789")),
    )
    .await
    .expect("Multi-range without Content-Type must not panic");
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    let bytes = body::to_bytes(response.into_body(), 1024)
        .await
        .expect("Body");
    let text = std::str::from_utf8(&bytes).expect("ASCII multipart");
    assert!(text.contains("Content-Range: bytes 0-1/10\r\n\r\n01"));
    assert!(text.contains("Content-Range: bytes 4-5/10\r\n\r\n45"));
}
