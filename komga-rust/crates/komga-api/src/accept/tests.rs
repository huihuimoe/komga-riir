use axum::http::header::ACCEPT;
use axum::http::{HeaderMap, HeaderValue};

use super::{InvalidAccept, error_json_type, parse, prefers_pdf, produced};

fn accept(values: &[&str]) -> Result<bool, InvalidAccept> {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append(ACCEPT, HeaderValue::from_str(value).expect("Header value"));
    }
    parse(&headers).map(prefers_pdf)
}

/// Expected results from Spring 6.2 `MediaType.parseMediaTypes`, `MimeTypeUtils
/// .sortBySpecificity` and `isCompatibleWith`, as `getBookPageInternal` combines them.
#[test]
fn pdf_is_preferred_only_when_more_specific_than_any_image() {
    let cases = [
        (&[][..], false),
        (&[""], false),
        (&["*/*"], true),
        (&["*"], true),
        (&["application/*"], true),
        (&["application/PDF"], true),
        (&["application/*+pdf"], true),
        (&["image/*"], false),
        (&["text/html"], false),
        (&["image/jpeg, application/pdf"], false),
        (&["application/pdf, image/*"], true),
        (&["image/*, application/pdf"], true),
        (&["image/*;q=0.9, application/pdf;q=0.8"], false),
        (&["image/jpeg;q=0.1, application/pdf"], true),
        (&["image/jpeg", "application/pdf"], false),
        (&["text/html, application/*, */*;q=0.1"], true),
        (
            &["image/avif,image/webp,image/apng,image/svg+xml,image/*,*/*;q=0.8"],
            false,
        ),
        (&["application/json, application/pdf"], true),
        (&["image/png;a=1;b=2, image/png;a=1, application/*"], false),
    ];
    for (values, expected) in cases {
        assert_eq!(accept(values), Ok(expected), "{values:?}");
    }
}

#[test]
fn unparseable_ranges_are_rejected() {
    let invalid = [
        "application",
        "application/",
        "*/pdf",
        "appli cation/pdf",
        "application/pdf;q=2",
        "application/pdf;q=x",
        "application/pdf;=1",
        "application/pdf;a=",
        "application/pdf;a=b c",
        "a/b=c",
    ];
    for value in invalid {
        assert!(accept(&[value]).is_err(), "{value}");
    }
    assert_eq!(accept(&["application/pdf;a=\"b c\""]), Ok(true));
    assert_eq!(accept(&["application/pdf;flag"]), Ok(true));
    assert!(accept(&[&["image/png"; 51].join(",")]).is_err());
    assert_eq!(accept(&[&["image/png"; 50].join(",")]), Ok(false));
}

#[test]
fn the_most_specific_acceptable_range_picks_the_produced_type() {
    let produces = ["application/atom+xml", "application/xml", "text/xml"];
    let cases = [
        (&[][..], Some("application/atom+xml")),
        (&["*/*"], Some("application/atom+xml")),
        (&["*/*, application/xml"], Some("application/xml")),
        (&["text/*"], Some("text/xml")),
        (&["application/*+xml"], Some("application/atom+xml")),
        (
            &["text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"],
            Some("application/xml"),
        ),
        (&["text/html"], None),
    ];
    for (values, expected) in cases {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(ACCEPT, HeaderValue::from_str(value).expect("Header value"));
        }
        let ranges = parse(&headers).expect("Parsed");
        assert_eq!(produced(ranges, &produces), expected, "{values:?}");
    }
}

#[test]
fn error_bodies_take_the_most_specific_json_range() {
    let cases = [
        ("", Some("application/json")),
        ("*/*", Some("application/json")),
        ("application/*", Some("application/json")),
        ("application/problem+json", Some("application/problem+json")),
        (
            "image/*, application/vnd.koreader.v1+json;q=0.1",
            Some("application/vnd.koreader.v1+json"),
        ),
        ("text/plain", None),
        ("image/jpeg", None),
    ];
    for (value, expected) in cases {
        let mut headers = HeaderMap::new();
        if !value.is_empty() {
            headers.insert(ACCEPT, HeaderValue::from_static(value));
        }
        let ranges = parse(&headers).expect("Parsed");
        assert_eq!(error_json_type(ranges).as_deref(), expected, "{value}");
    }
}
