//! Spring `ServletUriComponentsBuilder.fromCurrentContextPath()` behind `ForwardedHeaderFilter`
//! (`server.forward-headers-strategy: framework`), and the path encoding of
//! `UriComponentsBuilder.toUriString()`.

mod forwarded;
mod trusted_proxies;

use std::borrow::Cow;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::connect_info::{ConnectInfo, Connected};
use axum::extract::{FromRequestParts, OriginalUri, Request, State};
use axum::http::header::{HOST, LOCATION};
use axum::http::request::Parts;
use axum::http::{Extensions, HeaderMap, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::serve::IncomingStream;
use tokio::net::TcpListener;

use self::forwarded::{InvalidForwarded, Origin, first, forwarded_prefix};
pub use self::trusted_proxies::TrustedProxies;
use crate::error::ApiError;

const FORWARDED_HEADERS: [&str; 7] = [
    "forwarded",
    "x-forwarded-host",
    "x-forwarded-port",
    "x-forwarded-proto",
    "x-forwarded-prefix",
    "x-forwarded-ssl",
    "x-forwarded-for",
];

/// Connection addresses captured on accept: local socket for `Host`-less HTTP/1.0 fallback and
/// peer socket for verifying trusted reverse proxies.
#[derive(Clone, Copy, Debug)]
pub struct LocalAddress {
    local: Option<SocketAddr>,
    peer: Option<SocketAddr>,
}

impl Connected<IncomingStream<'_, TcpListener>> for LocalAddress {
    fn connect_info(stream: IncomingStream<'_, TcpListener>) -> Self {
        Self {
            local: stream.io().local_addr().ok(),
            peer: stream.io().peer_addr().ok(),
        }
    }
}

/// Configuration passed to [`layer`].
#[derive(Clone, Debug)]
pub(crate) struct LayerConfig {
    pub(crate) context_path: Arc<str>,
    pub(crate) trusted_proxies: TrustedProxies,
}

/// Client-visible request origin and context path (`ServletUriComponentsBuilder`).
#[derive(Clone, Debug)]
pub(crate) struct RequestContext(Arc<RequestContextInner>);

#[derive(Debug)]
struct RequestContextInner {
    origin: Origin,
    forwarded: bool,
    path: String,
}

/// Strips the configured context path (redirecting the bare root to `/`, 404 outside it) and
/// resolves forwarded headers from trusted proxies (400 if malformed) before handlers run.
pub(crate) async fn layer(
    State(config): State<LayerConfig>,
    mut request: Request,
    next: Next,
) -> Response {
    let context_path = &config.context_path;
    if !context_path.is_empty() {
        let uri = request.uri();
        match uri.path().strip_prefix(&**context_path) {
            Some("") => {
                let query = uri.query().map(|query| format!("?{query}"));
                let location = format!("{context_path}/{}", query.unwrap_or_default());
                return (StatusCode::FOUND, [(LOCATION, location)]).into_response();
            }
            Some(path) if path.starts_with('/') => {
                let path_and_query = match uri.query() {
                    Some(query) => format!("{path}?{query}"),
                    None => path.to_owned(),
                };
                *request.uri_mut() = path_and_query
                    .parse()
                    .expect("A suffix of a request path is a valid path");
            }
            _ => return StatusCode::NOT_FOUND.into_response(),
        }
    }
    let addresses = request
        .extensions()
        .get::<ConnectInfo<LocalAddress>>()
        .map(|ConnectInfo(addresses)| *addresses);
    let local = addresses.and_then(|addresses| addresses.local);
    let peer = addresses.and_then(|addresses| addresses.peer.map(|addr| addr.ip()));
    let trust_forwarded = config.trusted_proxies.is_trusted(peer);
    match RequestContext::of(
        request.headers(),
        request.uri(),
        local,
        context_path,
        trust_forwarded,
    ) {
        Ok(context) => {
            request.extensions_mut().insert(context);
            next.run(request).await
        }
        Err(InvalidForwarded) => ApiError::new(
            StatusCode::BAD_REQUEST,
            "No message available",
            original_path(&request),
        )
        .into_response(),
    }
}

/// The full request path, context path included, as error bodies name it.
pub(crate) fn original_path(request: &Request) -> &str {
    let original = request.extensions().get::<OriginalUri>();
    original.map_or_else(|| request.uri().path(), |OriginalUri(uri)| uri.path())
}

impl<S: Send + Sync> FromRequestParts<S> for RequestContext {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::from_extensions(&parts.extensions))
    }
}

impl RequestContext {
    pub(crate) fn from_extensions(extensions: &Extensions) -> Self {
        extensions
            .get::<Self>()
            .cloned()
            .expect("the router reads the request context before any handler")
    }

    fn of(
        headers: &HeaderMap,
        uri: &Uri,
        local: Option<SocketAddr>,
        context_path: &str,
        trust_forwarded: bool,
    ) -> Result<Self, InvalidForwarded> {
        let mut origin = Origin {
            scheme: Cow::Borrowed("http"),
            host: String::new(),
            port: None,
        };
        match first(headers, HOST).or_else(|| uri.authority().map(|authority| authority.as_str())) {
            Some(authority) => origin.set_host(authority)?,
            None => {
                let local = local.ok_or(InvalidForwarded)?;
                origin.host = match local {
                    SocketAddr::V4(address) => address.ip().to_string(),
                    SocketAddr::V6(address) => format!("[{}]", address.ip()),
                };
                origin.port = Some(i32::from(local.port()));
            }
        }
        origin.reset_default_port();
        if trust_forwarded {
            origin.adapt_forwarded(headers)?;
            origin.reset_default_port();
        }
        // `ForwardedPrefixExtractor`: the prefix replaces the context path (trusted proxies only).
        // `HierarchicalUriComponents.toUriString` separates a relative path from the host.
        let prefix = trust_forwarded.then(|| forwarded_prefix(headers)).flatten();
        let path = prefix.map_or_else(
            || context_path.to_owned(),
            |prefix| {
                if prefix.is_empty() || prefix.starts_with('/') {
                    encode_path(&prefix)
                } else {
                    format!("/{}", encode_path(&prefix))
                }
            },
        );
        let forwarded = trust_forwarded
            && FORWARDED_HEADERS
                .iter()
                .any(|name| headers.contains_key(*name));
        Ok(Self(Arc::new(RequestContextInner {
            origin,
            forwarded,
            path,
        })))
    }

    pub(crate) fn url(&self) -> String {
        format!("{}{}", self.0.origin, self.0.path)
    }

    pub(crate) fn path(&self) -> &str {
        &self.0.path
    }

    pub(crate) fn secure(&self) -> bool {
        self.0.origin.scheme.eq_ignore_ascii_case("https")
    }

    /// Kobo devices omit the port from `Host`; fall back to `port` unless proxied.
    pub(crate) fn kobo_url(&self, port: i32) -> String {
        if self.0.forwarded {
            return self.url();
        }
        let mut origin = self.0.origin.clone();
        origin.port = Some(port);
        origin.reset_default_port();
        format!("{origin}{}", self.0.path)
    }
}

/// Percent-encodes path bytes outside `pchar` and `/` after collapsing consecutive slashes.
pub(crate) fn encode_path(path: &str) -> String {
    encode(&sanitize(path), b"-._~!$&'()*+,;=:@/")
}

/// Percent-encodes a single URI path segment (`/` is encoded as `%2F`).
pub(crate) fn encode_path_segment(segment: &str) -> String {
    encode(segment, b"-._~!$&'()*+,;=:@")
}

/// Percent-encodes query parameter values (`&` and `=` encoded, `?` and `+` literal).
pub(crate) fn encode_query_param(value: &str) -> String {
    encode(value, b"-._~!$'()*+,;:@/?")
}

fn encode(value: &str, allowed: &[u8]) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || allowed.contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn sanitize(path: &str) -> String {
    let mut sanitized = String::with_capacity(path.len());
    for character in path.chars() {
        if !(character == '/' && sanitized.ends_with('/')) {
            sanitized.push(character);
        }
    }
    sanitized
}

#[cfg(test)]
mod tests;
