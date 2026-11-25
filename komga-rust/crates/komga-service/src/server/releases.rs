//! `ReleaseController`: the latest releases on GitHub, cached.

use std::time::Duration;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::support::remote_json::{FetchError, RemoteJson};

/// `GithubReleaseDto`.
#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    html_url: String,
    tag_name: String,
    published_at: Timestamp,
    body: String,
    prerelease: bool,
}

/// `ReleaseDto`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub release_date: Timestamp,
    pub url: String,
    /// The first, most recent release.
    pub latest: bool,
    pub pre_release: bool,
    pub description: String,
}

#[derive(Clone)]
pub struct ReleaseService {
    releases: RemoteJson<Vec<GithubRelease>>,
}

impl ReleaseService {
    /// `url` lists the releases, newest first (GitHub's `releases?per_page=20`).
    #[must_use]
    pub fn new(url: String, timeout: Duration) -> Self {
        Self {
            releases: RemoteJson::new(url, timeout),
        }
    }

    /// No releases for an empty body.
    pub async fn releases(&self) -> Result<Vec<Release>, FetchError> {
        let releases = self.releases.get().await?.unwrap_or_default();
        Ok(releases
            .into_iter()
            .enumerate()
            .map(|(index, release)| Release {
                version: release.tag_name,
                release_date: release.published_at,
                url: release.html_url,
                latest: index == 0,
                pre_release: release.prerelease,
                description: release.body,
            })
            .collect())
    }
}
