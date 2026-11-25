//! Spring's `produces` condition: 406 Not Acceptable when no `Accept` range takes what a route
//! produces.

use reqwest::header::{ACCEPT, ALLOW, CACHE_CONTROL};
use reqwest::{Method, StatusCode};

use crate::support::{TestServer, json};

#[tokio::test]
async fn routes_reject_requests_accepting_none_of_their_types() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let server = TestServer::start_claimed(config_dir.path()).await;
    let get = |path: &str, accept: &str| {
        server
            .admin(Method::GET, path)
            .header(ACCEPT, accept)
            .send()
    };
    let accept_header = |response: &reqwest::Response| {
        response.headers()[ACCEPT]
            .to_str()
            .expect("ASCII header")
            .to_owned()
    };

    for accept in ["text/plain", "bogus"] {
        let rejected = get("/api/v1/libraries", accept).await.expect("Request");
        assert_eq!(rejected.status(), StatusCode::NOT_ACCEPTABLE, "{accept}");
        assert_eq!(accept_header(&rejected), "application/json");
        assert!(rejected.headers().get(CACHE_CONTROL).is_none());
        assert!(rejected.bytes().await.expect("Body").is_empty(), "{accept}");
    }
    // Rejected before the path variables are bound.
    let unbound = get("/api/v1/books/nope/thumbnail", "application/json").await;
    assert_eq!(
        unbound.expect("Request").status(),
        StatusCode::NOT_ACCEPTABLE
    );

    let feed = get("/opds/v1.2/catalog", "application/json")
        .await
        .expect("Request");
    assert_eq!(feed.status(), StatusCode::NOT_ACCEPTABLE);
    assert_eq!(
        accept_header(&feed),
        "application/atom+xml, application/xml, text/xml"
    );
    let error = json(feed).await;
    assert_eq!(
        error["message"],
        "Acceptable representations: [application/atom+xml, application/xml, text/xml]."
    );
    assert_eq!(error["path"], "/opds/v1.2/catalog");

    // Authentication and the method come first.
    let anonymous = server
        .request(Method::GET, "/api/v1/libraries")
        .header(ACCEPT, "text/plain")
        .send()
        .await
        .expect("Request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let wrong_method = server
        .admin(Method::DELETE, "/api/v1/libraries")
        .header(ACCEPT, "text/plain")
        .send()
        .await
        .expect("Request");
    assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);

    // A page takes any type, a font stylesheet falls back to the font file route.
    let page = get("/api/v1/books/nope/pages/1", "text/plain").await;
    assert_eq!(page.expect("Request").status(), StatusCode::NOT_FOUND);
    let css = get(
        "/api/v1/fonts/resource/OpenDyslexic/css",
        "application/json",
    )
    .await;
    assert_eq!(css.expect("Request").status(), StatusCode::NOT_FOUND);

    // `consumes` comes before `produces`.
    let not_multipart = server
        .admin(Method::POST, "/api/v1/series/nope/thumbnails")
        .header(ACCEPT, "bogus")
        .send()
        .await
        .expect("Request");
    assert_eq!(not_multipart.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let unmatched = server
        .admin(Method::POST, "/api/v1/nope")
        .send()
        .await
        .expect("Request");
    assert_eq!(unmatched.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        json(unmatched).await["message"],
        "No endpoint POST /api/v1/nope."
    );
}

#[tokio::test]
async fn options_lists_the_mapped_methods_and_others_are_405() {
    let config_dir = komga_test_support::tempdir().expect("Config directory");
    let server = TestServer::start_claimed(config_dir.path()).await;
    let send = |method: Method, path: &str| server.admin(method, path).send();

    for (path, allow) in [
        ("/api/v1/libraries", "GET,HEAD,POST,OPTIONS"),
        ("/foo/bar", "GET,HEAD,OPTIONS"),
    ] {
        let options = send(Method::OPTIONS, path).await.expect("Request");
        assert_eq!(options.status(), StatusCode::OK, "{path}");
        assert_eq!(options.headers()[ALLOW], allow, "{path}");
    }
    let options = send(Method::OPTIONS, "/api/v1/nope")
        .await
        .expect("Request");
    assert_eq!(options.status(), StatusCode::NOT_FOUND);

    for (path, allow) in [("/api/v1/libraries", "GET,HEAD,POST"), ("/foo/bar", "GET")] {
        let rejected = send(Method::DELETE, path).await.expect("Request");
        assert_eq!(rejected.status(), StatusCode::METHOD_NOT_ALLOWED, "{path}");
        assert_eq!(rejected.headers()[ALLOW], allow, "{path}");
        let error = json(rejected).await;
        assert_eq!(error["message"], "Method 'DELETE' is not supported.");
        assert_eq!(error["path"], path);
    }
}
