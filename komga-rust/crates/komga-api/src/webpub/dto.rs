//! Kotlin `WepPub.kt` DTOs, serialized as Komga's Jackson mapper does. Properties the generators
//! never set are left out.

use jiff::civil::Date;
use serde::Serialize;

/// `WPPublicationDto`, `NON_NULL`: empty lists are kept.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WpPublication {
    /// Serialized as `context`: `@JsonAlias("@context")` only names it when reading.
    pub context: &'static str,
    pub metadata: WpMetadata,
    pub links: Vec<WpLink>,
    pub images: Vec<WpLink>,
    pub reading_order: Vec<WpLink>,
    pub resources: Vec<WpLink>,
    pub toc: Vec<WpLink>,
    pub landmarks: Vec<WpLink>,
    pub page_list: Vec<WpLink>,
}

/// `WPMetadataDto`, `NON_EMPTY`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WpMetadata {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conforms_to: Option<&'static str>,
    /// ISO date time with the server's offset.
    pub modified: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published: Option<Date>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub language: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub author: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub translator: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub editor: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artist: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub illustrator: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub letterer: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub penciler: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub colorist: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inker: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contributor: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subject: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading_progression: Option<WpReadingProgression>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub number_of_pages: i32,
    pub belongs_to: WpBelongsTo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rendition: Option<WpRendition>,
}

/// `WPBelongsToDto`.
#[derive(Debug, Serialize)]
pub struct WpBelongsTo {
    pub series: Vec<WpContributor>,
}

/// `WPContributorDto`, `NON_EMPTY`.
#[derive(Debug, Serialize)]
pub struct WpContributor {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub position: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<WpLink>,
}

/// The `rendition` map of EPUB manifests.
#[derive(Debug, Serialize)]
pub struct WpRendition {
    pub layout: &'static str,
}

/// `WPReadingProgressionDto`.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WpReadingProgression {
    Rtl,
    Ltr,
    Ttb,
}

/// `WPLinkDto`, `NON_EMPTY`.
#[derive(Debug, Default, Serialize)]
pub struct WpLink {
    #[serde(skip_serializing_if = "is_none_or_empty")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rel: Option<&'static str>,
    #[serde(skip_serializing_if = "is_none_or_empty")]
    pub href: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "is_none_or_empty")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub templated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternate: Vec<WpLink>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<WpLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<WpLinkProperties>,
}

/// The only link `properties` Komga writes: where OPDS clients authenticate.
#[derive(Debug, Clone, Serialize)]
pub struct WpLinkProperties {
    pub authenticate: WpAuthenticate,
}

#[derive(Debug, Clone, Serialize)]
pub struct WpAuthenticate {
    pub href: String,
    #[serde(rename = "type")]
    pub media_type: &'static str,
}

#[expect(clippy::ref_option, reason = "serde passes a reference to the field")]
fn is_none_or_empty(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(str::is_empty)
}
