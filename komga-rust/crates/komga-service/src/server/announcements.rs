//! `AnnouncementController`: the Komga website's news feed, cached, with each user's read state.

use std::collections::BTreeSet;
use std::time::Duration;

use jiff::Timestamp;
use komga_db::{AnnouncementsReadDao, Database, DbError};
use serde::{Deserialize, Serialize};

use crate::support::remote_json::{FetchError, RemoteJson};

/// A JSON Feed (`JsonFeedDto`), keeping the fields Komga shows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonFeed {
    pub version: String,
    pub title: String,
    pub home_page_url: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub items: Vec<FeedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedItem {
    pub id: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub content_html: Option<String>,
    pub date_modified: Option<Timestamp>,
    pub author: Option<FeedAuthor>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Whether the requesting user marked the item read; any remote value is replaced.
    #[serde(rename = "_komga")]
    pub komga: Option<KomgaExtension>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedAuthor {
    pub name: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct KomgaExtension {
    pub read: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum AnnouncementError {
    /// The website answered with an empty body.
    #[error("The announcements feed is empty")]
    Empty,
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error(transparent)]
    Db(#[from] DbError),
}

#[derive(Clone)]
pub struct AnnouncementService {
    db: Database,
    feed: RemoteJson<JsonFeed>,
}

impl AnnouncementService {
    #[must_use]
    pub fn new(db: Database, url: String, timeout: Duration) -> Self {
        Self {
            db,
            feed: RemoteJson::new(url, timeout),
        }
    }

    /// The feed with the user's read state.
    pub async fn announcements(&self, user_id: &str) -> Result<JsonFeed, AnnouncementError> {
        let mut feed = self.feed.get().await?.ok_or(AnnouncementError::Empty)?;
        let read = AnnouncementsReadDao::find_ids(&self.db, user_id).await?;
        for item in &mut feed.items {
            item.komga = Some(KomgaExtension {
                read: read.contains(&item.id),
            });
        }
        Ok(feed)
    }

    pub async fn mark_read(&self, user_id: &str, ids: &BTreeSet<String>) -> Result<(), DbError> {
        let mut transaction = self.db.begin().await?;
        AnnouncementsReadDao::insert(&mut transaction, user_id, ids).await?;
        transaction.commit().await
    }
}
