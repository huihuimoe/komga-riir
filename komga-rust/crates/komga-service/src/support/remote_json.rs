//! A JSON document fetched from one fixed URL and cached, as the Kotlin controllers do with
//! WebClient and Caffeine. The URL is not caller-chosen, so this is not a proxy.

use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// The document is fetched again once it has not been asked for in this long.
const CACHE_EXPIRY: Duration = Duration::from_secs(60 * 60);
/// Spring's `spring.http.codecs.max-in-memory-size`.
const MAX_SIZE: usize = 10 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("Failed to fetch {url}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("The document at {url} is larger than {MAX_SIZE} bytes")]
    TooLarge { url: String },
    #[error("Failed to parse the document at {url}")]
    Parse {
        url: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Clone)]
pub(crate) struct RemoteJson<T> {
    client: reqwest::Client,
    url: String,
    cache: Arc<Mutex<Option<Cached<T>>>>,
}

struct Cached<T> {
    value: Option<T>,
    last_access: Instant,
}

impl<T: DeserializeOwned + Clone> RemoteJson<T> {
    /// `timeout` bounds each whole fetch, which Kotlin leaves unbounded.
    pub(crate) fn new(url: String, timeout: Duration) -> Self {
        let client = komga_oauth::http_client(|builder| {
            builder
                .timeout(timeout)
                // GitHub rejects requests without one.
                .user_agent("Komga")
        });
        Self {
            client,
            url,
            cache: Arc::default(),
        }
    }

    /// The cached document, else a fresh one; `None` for an empty body. An empty body is cached
    /// like a document, so that GitHub's rate limit is not spent on it; a failure is not.
    ///
    /// Holds the cache while fetching, so that concurrent requests share one fetch.
    pub(crate) async fn get(&self) -> Result<Option<T>, FetchError> {
        let mut cache = self.cache.lock().await;
        let now = Instant::now();
        if let Some(cached) = cache
            .as_mut()
            .filter(|cached| now.duration_since(cached.last_access) < CACHE_EXPIRY)
        {
            cached.last_access = now;
            return Ok(cached.value.clone());
        }
        let value = self.fetch().await?;
        *cache = Some(Cached {
            value: value.clone(),
            last_access: now,
        });
        Ok(value)
    }

    async fn fetch(&self) -> Result<Option<T>, FetchError> {
        let request = |source| FetchError::Request {
            url: self.url.clone(),
            source,
        };
        let mut response = self
            .client
            .get(&self.url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(request)?;
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request)? {
            if body.len() + chunk.len() > MAX_SIZE {
                return Err(FetchError::TooLarge {
                    url: self.url.clone(),
                });
            }
            body.extend_from_slice(&chunk);
        }
        if body.is_empty() {
            return Ok(None);
        }
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|source| FetchError::Parse {
                url: self.url.clone(),
                source,
            })
    }
}
