//! Parsing of `Origin`, `Forwarded`, and `X-Forwarded-*` headers (Spring `ForwardedHeaderFilter`).

use std::borrow::Cow;

use axum::http::HeaderMap;

#[derive(Debug)]
pub(super) struct InvalidForwarded;

#[derive(Clone, Debug)]
pub(super) struct Origin {
    pub(super) scheme: Cow<'static, str>,
    pub(super) host: String,
    pub(super) port: Option<i32>,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secure = matches!(self.scheme.as_ref(), "https" | "wss");
        let port = self.port.unwrap_or(if secure { 443 } else { 80 });
        write!(formatter, "{}://{}", self.scheme, self.host)?;
        match self.scheme.as_ref() {
            "http" if port == 80 => Ok(()),
            "https" if port == 443 => Ok(()),
            _ => write!(formatter, ":{port}"),
        }
    }
}

impl Origin {
    pub(super) fn adapt_forwarded(&mut self, headers: &HeaderMap) -> Result<(), InvalidForwarded> {
        if let Some(forwarded) = non_empty(headers, "forwarded") {
            if let Some(proto) = forwarded_value(forwarded, "proto") {
                self.set_scheme(proto.trim());
            } else if is_forwarded_ssl_on(headers) {
                self.set_scheme("https");
            }
            if let Some(host) = forwarded_value(forwarded, "host") {
                self.set_host(host.trim())?;
            }
            return Ok(());
        }
        if let Some(proto) = non_empty(headers, "x-forwarded-proto") {
            self.set_scheme(first_token(proto)?);
        } else if is_forwarded_ssl_on(headers) {
            self.set_scheme("https");
        }
        if let Some(host) = non_empty(headers, "x-forwarded-host") {
            self.set_host(first_token(host)?)?;
        }
        if let Some(port) = non_empty(headers, "x-forwarded-port") {
            self.port = Some(parse_int(first_token(port)?)?);
        }
        Ok(())
    }

    fn set_scheme(&mut self, scheme: &str) {
        self.scheme = match scheme {
            "http" => Cow::Borrowed("http"),
            "https" => Cow::Borrowed("https"),
            other => Cow::Owned(other.to_owned()),
        };
        self.port = None;
    }

    pub(super) fn set_host(&mut self, value: &str) -> Result<(), InvalidForwarded> {
        let port_separator = value.rfind(':');
        let square_bracket = value.rfind(']');
        match port_separator {
            Some(separator) if square_bracket.is_none_or(|bracket| separator > bracket) => {
                if square_bracket.is_none() && value.find(':') != Some(separator) {
                    return Err(InvalidForwarded);
                }
                value[..separator].clone_into(&mut self.host);
                self.port = Some(parse_int(&value[separator + 1..])?);
            }
            _ => {
                value.clone_into(&mut self.host);
                self.port = None;
            }
        }
        Ok(())
    }

    pub(super) fn reset_default_port(&mut self) {
        match (self.scheme.as_ref(), self.port) {
            ("http", Some(80)) | ("https", Some(443)) => self.port = None,
            _ => {}
        }
    }
}

pub(super) fn forwarded_prefix(headers: &HeaderMap) -> Option<String> {
    let value = first(headers, "x-forwarded-prefix")?;
    Some(
        tokens(value)
            .map(|part| part.trim_end_matches('/'))
            .collect(),
    )
}

/// Matches `(?i:<name>)="?([^;,"]+)"?` anywhere in a `Forwarded` header.
fn forwarded_value<'header>(header: &'header str, name: &str) -> Option<&'header str> {
    let lowercase = header.to_ascii_lowercase();
    let key = format!("{name}=");
    let mut from = 0;
    while let Some(found) = lowercase[from..].find(&key) {
        let start = from + found + key.len();
        let value = header[start..]
            .strip_prefix('"')
            .unwrap_or(&header[start..]);
        let end = value.find([';', ',', '"']).unwrap_or(value.len());
        if end > 0 {
            return Some(&value[..end]);
        }
        from = start;
    }
    None
}

fn is_forwarded_ssl_on(headers: &HeaderMap) -> bool {
    first(headers, "x-forwarded-ssl").is_some_and(|value| value.eq_ignore_ascii_case("on"))
}

pub(super) fn first(
    headers: &HeaderMap,
    name: impl axum::http::header::AsHeaderName,
) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn non_empty(headers: &HeaderMap, name: impl axum::http::header::AsHeaderName) -> Option<&str> {
    first(headers, name).filter(|value| !value.trim().is_empty())
}

fn tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn first_token(value: &str) -> Result<&str, InvalidForwarded> {
    tokens(value).next().ok_or(InvalidForwarded)
}

fn parse_int(value: &str) -> Result<i32, InvalidForwarded> {
    value.parse().map_err(|_| InvalidForwarded)
}
