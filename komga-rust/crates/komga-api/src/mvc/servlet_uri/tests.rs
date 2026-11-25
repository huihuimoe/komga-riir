use axum::http::{HeaderMap, HeaderName, HeaderValue, Uri};

use super::{RequestContext, encode_path};

fn url(pairs: &[(&str, &str)]) -> Option<String> {
    url_at("", pairs)
}

fn url_at(context_path: &str, pairs: &[(&str, &str)]) -> Option<String> {
    url_with_trust(context_path, pairs, true)
}

fn url_with_trust(
    context_path: &str,
    pairs: &[(&str, &str)],
    trust_forwarded: bool,
) -> Option<String> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).expect("Header name"),
            HeaderValue::from_str(value).expect("Header value"),
        );
    }
    RequestContext::of(
        &headers,
        &Uri::from_static("/api/v1/books/1/manifest"),
        None,
        context_path,
        trust_forwarded,
    )
    .ok()
    .map(|context| context.url())
}

#[test]
fn the_context_url_follows_spring_forwarded_header_rules() {
    let host = ("host", "komga.local:25600");
    for (pairs, expected) in [
        (vec![host], Some("http://komga.local:25600")),
        (vec![("host", "komga.local:80")], Some("http://komga.local")),
        (vec![("host", "[::1]:8080")], Some("http://[::1]:8080")),
        (vec![("host", "a:b:c")], None),
        (
            vec![host, ("x-forwarded-proto", "https")],
            Some("https://komga.local"),
        ),
        (
            vec![
                host,
                ("x-forwarded-proto", " https , http"),
                ("x-forwarded-port", "8443"),
            ],
            Some("https://komga.local:8443"),
        ),
        (
            vec![host, ("x-forwarded-ssl", "ON")],
            Some("https://komga.local"),
        ),
        (
            vec![host, ("x-forwarded-host", "proxy.example:9000, other")],
            Some("http://proxy.example:9000"),
        ),
        (
            vec![host, ("x-forwarded-host", "proxy.example")],
            Some("http://proxy.example"),
        ),
        (vec![host, ("x-forwarded-port", "x")], None),
        (vec![host, ("x-forwarded-proto", ",")], None),
        (
            vec![host, ("x-forwarded-proto", "HTTPS")],
            Some("HTTPS://komga.local:80"),
        ),
        // `Forwarded` wins over every `X-Forwarded-*` header but `X-Forwarded-Ssl`.
        (
            vec![
                host,
                (
                    "forwarded",
                    "for=1.2.3.4;Proto=\"https\";host=proxy.example",
                ),
                ("x-forwarded-host", "ignored"),
            ],
            Some("https://proxy.example"),
        ),
        (
            vec![
                host,
                ("forwarded", "host=\"\";host=b.example"),
                ("x-forwarded-ssl", "on"),
            ],
            Some("https://b.example"),
        ),
        (
            vec![host, ("x-forwarded-prefix", "/komga/, /library//")],
            Some("http://komga.local:25600/komga/library"),
        ),
        (
            vec![host, ("x-forwarded-prefix", "my books")],
            Some("http://komga.local:25600/my%20books"),
        ),
        (
            vec![host, ("x-forwarded-prefix", "/")],
            Some("http://komga.local:25600"),
        ),
    ] {
        assert_eq!(url(&pairs).as_deref(), expected, "{pairs:?}");
    }
}

#[test]
fn the_forwarded_prefix_replaces_the_context_path() {
    let host = ("host", "komga.local");
    assert_eq!(
        url_at("/komga", &[host]).as_deref(),
        Some("http://komga.local/komga")
    );
    assert_eq!(
        url_at("/komga", &[host, ("x-forwarded-prefix", "/proxy")]).as_deref(),
        Some("http://komga.local/proxy")
    );
}

#[test]
fn paths_are_encoded_like_spring() {
    assert_eq!(
        encode_path("/books/1/resource/OPS//a b#c%é@:!$&'()*+,;=~.xhtml"),
        "/books/1/resource/OPS/a%20b%23c%25%C3%A9@:!$&'()*+,;=~.xhtml"
    );
}

#[test]
fn untrusted_peers_ignore_forwarded_headers() {
    let headers = [
        ("host", "komga.local:25600"),
        ("x-forwarded-proto", "https"),
        ("x-forwarded-host", "evil.example"),
        ("x-forwarded-prefix", "/spoofed"),
    ];
    assert_eq!(
        url_with_trust("/komga", &headers, false).as_deref(),
        Some("http://komga.local:25600/komga")
    );
}
