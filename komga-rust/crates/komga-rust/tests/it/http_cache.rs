//! Kotlin's `WebContentInterceptor` (`cachePrivate`) and `ShallowEtagHeaderFilter`.

use reqwest::header::{CACHE_CONTROL, CONTENT_LENGTH, ETAG, HeaderName, IF_NONE_MATCH, RANGE};
use reqwest::{Method, RequestBuilder, Response, StatusCode};

use crate::support::TestServer;

const CACHE_PRIVATE: &str = "max-age=0, must-revalidate, private";

fn header(response: &Response, name: &HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .map(|value| value.to_str().expect("ASCII header").to_owned())
}

async fn send(request: RequestBuilder) -> Response {
    request.send().await.expect("Request failed")
}

#[tokio::test]
async fn handler_responses_are_private_and_get_body_etags() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let server = TestServer::start_claimed(config_dir.path()).await;
    let me = || server.admin(Method::GET, "/api/v2/users/me");

    let first = send(me()).await;
    assert_eq!(
        header(&first, &CACHE_CONTROL).as_deref(),
        Some(CACHE_PRIVATE)
    );
    let etag = header(&first, &ETAG).expect("ETag");
    let digest = etag
        .strip_prefix("\"0")
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("Spring's quoted `0<md5>` tag");
    assert_eq!(digest.len(), 32, "{etag}");
    let body = first.bytes().await.expect("Body");

    for requested in [
        etag.clone(),
        format!("W/{etag}"),
        format!("\"other\", {etag}"),
    ] {
        let cached = send(me().header(IF_NONE_MATCH, &requested)).await;
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED, "{requested}");
        assert_eq!(header(&cached, &ETAG).as_deref(), Some(etag.as_str()));
        assert_eq!(
            header(&cached, &CACHE_CONTROL).as_deref(),
            Some(CACHE_PRIVATE)
        );
        assert_eq!(header(&cached, &CONTENT_LENGTH), None, "{requested}");
    }
    for requested in ["*", "\"other\""] {
        let fresh = send(me().header(IF_NONE_MATCH, requested)).await;
        assert_eq!(fresh.status(), StatusCode::OK, "{requested}");
        assert_eq!(fresh.bytes().await.expect("Body"), body, "{requested}");
    }

    let head = send(server.admin(Method::HEAD, "/api/v2/users/me")).await;
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(header(&head, &ETAG), None, "Only GET gets an ETag");
    assert_eq!(
        header(&head, &CACHE_CONTROL).as_deref(),
        Some(CACHE_PRIVATE)
    );

    // A controller error passed the interceptor but is not a 2xx.
    let missing = send(server.request(Method::GET, "/api/v1/fonts/resource/None/css")).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        header(&missing, &CACHE_CONTROL).as_deref(),
        Some(CACHE_PRIVATE)
    );
    assert_eq!(header(&missing, &ETAG), None);

    // A 206 gets an ETag, but only a 200 turns into a 304.
    let font = "/api/v1/fonts/resource/OpenDyslexic/OpenDyslexic-Bold.woff";
    let partial = send(server.request(Method::GET, font).header(RANGE, "bytes=0-3")).await;
    assert_eq!(partial.status(), StatusCode::PARTIAL_CONTENT);
    let partial_tag = header(&partial, &ETAG).expect("Partial ETag");
    let again = server
        .request(Method::GET, font)
        .header(RANGE, "bytes=0-3")
        .header(IF_NONE_MATCH, &partial_tag);
    assert_eq!(send(again).await.status(), StatusCode::PARTIAL_CONTENT);
    server.stop().await;
}

#[tokio::test]
async fn security_filter_responses_skip_the_cache_headers() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let server = TestServer::start_claimed(config_dir.path()).await;

    let unauthorized = send(server.request(Method::GET, "/api/v2/users/me")).await;
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(header(&unauthorized, &CACHE_CONTROL), None);
    assert_eq!(header(&unauthorized, &ETAG), None);

    let logout = send(server.admin(Method::GET, "/api/logout")).await;
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert_eq!(header(&logout, &CACHE_CONTROL), None);
    assert_eq!(header(&logout, &ETAG), None);
    server.stop().await;
}
