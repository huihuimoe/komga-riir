//! The two web UIs: Kotlin `IndexController`, the static resources of `WebMvcConfiguration`, and
//! `ResourceNotFoundController`, which sends unmatched page navigations to the UI and leaves
//! every other unmatched request a real 404.

use std::borrow::Cow;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{OriginalUri, State};
use axum::http::header::{self, CACHE_CONTROL, CONTENT_TYPE, LAST_MODIFIED};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::middleware::from_fn_with_state;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{any, get};
use komga_service::ServerSettingsService;

use crate::auth::authenticate_request;
use crate::error::ApiError;
use crate::mvc::http_cache::under;
use crate::mvc::{not_modified, resource};
use crate::routes::ApiState;

/// A file of a built UI.
pub struct WebAsset {
    pub data: Cow<'static, [u8]>,
    pub mime: Cow<'static, str>,
    /// Seconds since the epoch.
    pub last_modified: Option<u64>,
}

/// Finds a UI file by its path relative to the UI's root, without a leading `/`.
pub type AssetLookup = fn(&str) -> Option<WebAsset>;

/// The built UIs; a UI that was not built finds no files.
#[derive(Clone, Copy)]
pub struct WebUi {
    pub classic: AssetLookup,
    pub next: AssetLookup,
}

impl WebUi {
    /// The next UI's files replace the classic UI's files of the same name, as in Kotlin's build.
    fn asset(self, name: &str) -> Option<WebAsset> {
        (self.next)(name).or_else(|| (self.classic)(name))
    }
}

#[derive(Clone, Copy)]
enum Ui {
    Classic,
    Next,
}

#[derive(Clone)]
struct WebUiState {
    ui: WebUi,
    settings: ServerSettingsService,
    /// `IndexController.baseUrl`: the context path and `/`.
    base_url: Arc<str>,
}

/// Content-hashed directories, cached for a year; every other file is `no-store`.
const IMMUTABLE_ROOTS: [&str; 5] = ["/css", "/fonts", "/img", "/js", "/assets"];
const IMMUTABLE: &str = "max-age=31536000, public";
const NO_STORE: &str = "no-store";

/// Unmatched paths below these roots are 404 for authenticated users, like Kotlin, whose
/// security chain answers anonymous requests there first.
const AUTHENTICATED_ROOTS: [&str; 3] = ["/api", "/opds", "/sse"];

/// Protocol roots that are never UI pages. springdoc's are not served at all.
const PROTOCOL_ROOTS: [&str; 7] = [
    "/actuator",
    "/kobo",
    "/koreader",
    "/oauth2",
    "/login/oauth2",
    "/swagger-ui",
    "/v3",
];

/// The page routes and the fallback for everything no other router matches.
pub fn router(state: &ApiState) -> Router {
    let mut unmatched_api = Router::new();
    for root in AUTHENTICATED_ROOTS {
        unmatched_api = unmatched_api
            .route(root, any(not_found))
            .route(&format!("{root}/{{*rest}}"), any(not_found));
    }
    let unmatched_api =
        unmatched_api.route_layer(from_fn_with_state(state.auth.clone(), authenticate_request));
    Router::new()
        .route("/", get(default_page))
        .route("/index.html", get(classic_page))
        .route("/classic", get(classic_page))
        .route("/classic/", get(classic_page))
        .route("/classic/{*rest}", get(classic_page))
        .route("/index-next.html", get(next_page))
        .route("/next", get(next_page))
        .route("/next/", get(next_page))
        .route("/next/{*rest}", get(next_page))
        .fallback(fallback)
        .with_state(WebUiState {
            ui: state.web_ui,
            settings: state.settings.clone(),
            base_url: format!("{}/", state.web_server.context_path).into(),
        })
        .merge(unmatched_api)
}

/// Spring's `NoHandlerFoundException`.
async fn not_found(method: Method, OriginalUri(uri): OriginalUri) -> Response {
    let message = format!("No endpoint {method} {}.", uri.path());
    ApiError::new(StatusCode::NOT_FOUND, message, uri.path()).into_response()
}

async fn default_page(State(state): State<WebUiState>, uri: OriginalUri) -> Response {
    default_ui_page(&state, &uri).await
}

async fn classic_page(State(state): State<WebUiState>, uri: OriginalUri) -> Response {
    page(&state, Ui::Classic, &uri)
}

async fn next_page(State(state): State<WebUiState>, uri: OriginalUri) -> Response {
    page(&state, Ui::Next, &uri)
}

async fn default_ui_page(state: &WebUiState, uri: &OriginalUri) -> Response {
    match state.settings.load().await {
        Ok(settings) if settings.next_ui_as_default => page(state, Ui::Next, uri),
        Ok(_) => page(state, Ui::Classic, uri),
        Err(error) => ApiError::internal_error(uri.path(), error).into_response(),
    }
}

/// A UI's `index.html` rendered like Kotlin's Thymeleaf template.
fn page(state: &WebUiState, ui: Ui, OriginalUri(uri): &OriginalUri) -> Response {
    let lookup = match ui {
        Ui::Classic => state.ui.classic,
        Ui::Next => state.ui.next,
    };
    let Some(index) = lookup("index.html") else {
        return ApiError::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", uri.path()).into_response();
    };
    let html = render(&String::from_utf8_lossy(&index.data), &state.base_url);
    ([(CACHE_CONTROL, NO_STORE)], Html(html)).into_response()
}

async fn fallback(
    State(state): State<WebUiState>,
    method: Method,
    headers: HeaderMap,
    // Below the context path, unlike `original`.
    uri: Uri,
    original: OriginalUri,
) -> Response {
    let path = uri.path();
    if method == Method::GET || method == Method::HEAD {
        if let Some(asset) = path.strip_prefix('/').and_then(|name| state.ui.asset(name)) {
            return static_file(&headers, path, asset).await;
        }
        if is_page(path) {
            return default_ui_page(&state, &original).await;
        }
    } else if !PROTOCOL_ROOTS.iter().any(|root| under(path, root)) {
        return static_method_not_allowed(&method, &original);
    }
    not_found(method, original).await
}

/// Spring's static resource handler maps every other path for GET and HEAD; its 405 body carries
/// a charset, unlike the controllers' (see `mvc::method_not_allowed`).
fn static_method_not_allowed(method: &Method, OriginalUri(uri): &OriginalUri) -> Response {
    if method == Method::OPTIONS {
        let allow = HeaderValue::from_static("GET,HEAD,OPTIONS");
        return (StatusCode::OK, [(header::ALLOW, allow)]).into_response();
    }
    let message = format!("Method '{method}' is not supported.");
    let mut response =
        ApiError::new(StatusCode::METHOD_NOT_ALLOWED, message, uri.path()).into_response();
    let headers = response.headers_mut();
    headers.insert(header::ALLOW, HeaderValue::from_static("GET"));
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json;charset=UTF-8"),
    );
    response
}

/// A UI route: outside the protocol roots, and without a file extension.
fn is_page(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or_default();
    !file.contains('.') && !PROTOCOL_ROOTS.iter().any(|root| under(path, root))
}

async fn static_file(request: &HeaderMap, path: &str, asset: WebAsset) -> Response {
    let cache = if IMMUTABLE_ROOTS.iter().any(|root| under(path, root)) {
        IMMUTABLE
    } else {
        NO_STORE
    };
    let mut headers = HeaderMap::new();
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Some(seconds) = asset.last_modified {
        let millis = i64::try_from(seconds.saturating_mul(1000)).unwrap_or(i64::MAX);
        if not_modified::check(request, millis) {
            return (StatusCode::NOT_MODIFIED, headers).into_response();
        }
        let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
        let date = httpdate::fmt_http_date(time);
        headers.insert(
            LAST_MODIFIED,
            HeaderValue::from_str(&date).expect("HTTP dates are ASCII"),
        );
    }
    if let Ok(mime) = HeaderValue::from_str(&asset.mime) {
        headers.insert(CONTENT_TYPE, mime);
    }
    let data = match asset.data {
        Cow::Borrowed(data) => Bytes::from_static(data),
        Cow::Owned(data) => Bytes::from(data),
    };
    resource::response(request, headers, resource::Resource::Memory(data))
        .await
        .unwrap_or_else(|error| ApiError::internal_error(path, error).into_response())
}

/// The Thymeleaf expression that Kotlin's `index.html` templates set the base URL with.
const BASE_URL_EXPRESSION: &str = r#"/*[(${"'" + baseUrl + "'"})]*/ '/'"#;

/// Kotlin's build turns `src`, `content` and `href` values that start with a path, relative or
/// not, into `@{/…}` links resolved against the base URL; the template sets
/// `window.resourceBaseUrl` to it.
fn render(template: &str, base: &str) -> String {
    let template = template
        .replace(BASE_URL_EXPRESSION, &format!("'{base}'"))
        .replace(r#" th:inline="javascript""#, "");
    let mut html = String::with_capacity(template.len());
    let mut rest = template.as_str();
    while let Some(start) = rest.find("=\"") {
        let (head, tail) = rest.split_at(start + 2);
        html.push_str(head);
        let end = tail.find('"').unwrap_or(tail.len());
        let (value, tail) = tail.split_at(end);
        let attribute = &head[..head.len() - 2];
        if ["src", "content", "href"]
            .iter()
            .any(|name| attribute.ends_with(name))
            && is_path(value)
        {
            html.push_str(base);
            html.push_str(value.strip_prefix('/').unwrap_or(value));
        } else {
            html.push_str(value);
        }
        rest = tail;
    }
    html.push_str(rest);
    html
}

/// Kotlin's `[\w]*/.*?`: word characters up to a `/`; protocol-relative URLs are not paths.
fn is_path(value: &str) -> bool {
    value.find('/').is_some_and(|slash| {
        value[..slash]
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_')
            && !value.starts_with("//")
    })
}

#[cfg(test)]
mod tests {
    use super::{is_page, render};

    #[test]
    fn render_resolves_paths_against_the_base_url() {
        let template = r#"<meta content="width=device-width"><link href="/favicon.ico"><script th:inline="javascript">window.resourceBaseUrl = /*[(${"'" + baseUrl + "'"})]*/ '/'</script><script src="js/app.js"></script><a href="https://komga.org/">"#;
        assert_eq!(
            render(template, "/komga/"),
            r#"<meta content="width=device-width"><link href="/komga/favicon.ico"><script>window.resourceBaseUrl = '/komga/'</script><script src="/komga/js/app.js"></script><a href="https://komga.org/">"#
        );
    }

    #[test]
    fn pages_exclude_files_and_protocol_roots() {
        assert!(is_page("/series/0F99E2PSBK0FZ"));
        assert!(is_page("/"));
        assert!(!is_page("/mstile-150x150.png"));
        assert!(!is_page("/actuator/unknown"));
        assert!(!is_page("/kobo"));
    }
}
