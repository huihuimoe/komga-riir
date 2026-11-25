//! OPDS 2 feeds (Kotlin `Opds2Dto`).

use serde::Serialize;

use crate::webpub::{WpLink, WpPublication};

#[derive(Debug, Default, Serialize)]
pub(in crate::opds) struct Feed {
    pub(in crate::opds) metadata: FeedMetadata,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(in crate::opds) links: Vec<WpLink>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(in crate::opds) navigation: Vec<WpLink>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(in crate::opds) facets: Vec<Facet>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(in crate::opds) groups: Vec<Group>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(in crate::opds) publications: Vec<WpPublication>,
}

#[derive(Debug, Serialize)]
pub(in crate::opds) struct Facet {
    pub(in crate::opds) metadata: FeedMetadata,
    pub(in crate::opds) links: Vec<WpLink>,
}

#[derive(Debug, Serialize)]
pub(in crate::opds) struct Group {
    pub(in crate::opds) metadata: FeedMetadata,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(in crate::opds) links: Vec<WpLink>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(in crate::opds) navigation: Vec<WpLink>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(in crate::opds) publications: Vec<WpPublication>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::opds) struct FeedMetadata {
    pub(in crate::opds) title: String,
    /// ISO date time with the server's offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::opds) modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::opds) description: Option<String>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub(in crate::opds) page: Option<PageMetadata>,
}

/// The page a feed or group shows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::opds) struct PageMetadata {
    pub(in crate::opds) items_per_page: u32,
    /// From 1.
    pub(in crate::opds) current_page: u32,
    pub(in crate::opds) number_of_items: u64,
}

impl FeedMetadata {
    pub(in crate::opds) fn titled(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }
}
