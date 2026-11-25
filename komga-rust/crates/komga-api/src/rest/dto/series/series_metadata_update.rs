//! `SeriesMetadataUpdateDto` and its patch in `SeriesController.updateSeriesMetadata`: an absent
//! field keeps the stored value; `null` keeps the status, the text fields, the language, and the
//! locks, and unsets the other fields.

use std::str::FromStr;

use komga_service::SeriesMetadataUpdate;
use komga_types::{AlternateTitle, ReadingDirection, SeriesStatus, normalize_language_tag};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::error::Violation;
use crate::rest::dto::{
    WebLinkUpdateDto, not_blank_pairs, present, validated, web_link_violations, web_links,
};
use crate::rest::validation;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesMetadataUpdateDto {
    #[serde(default, deserialize_with = "named")]
    status: Option<SeriesStatus>,
    status_lock: Option<bool>,
    title: Option<String>,
    title_lock: Option<bool>,
    title_sort: Option<String>,
    title_sort_lock: Option<bool>,
    summary: Option<String>,
    summary_lock: Option<bool>,
    publisher: Option<String>,
    publisher_lock: Option<bool>,
    #[serde(default, deserialize_with = "present_named")]
    reading_direction: Option<Option<ReadingDirection>>,
    reading_direction_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    age_rating: Option<Option<i32>>,
    age_rating_lock: Option<bool>,
    language: Option<String>,
    language_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    genres: Option<Option<Vec<String>>>,
    genres_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    tags: Option<Option<Vec<String>>>,
    tags_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    total_book_count: Option<Option<i32>>,
    total_book_count_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    sharing_labels: Option<Option<Vec<String>>>,
    sharing_labels_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    links: Option<Option<Vec<WebLinkUpdateDto>>>,
    links_lock: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    alternate_titles: Option<Option<Vec<AlternateTitleUpdateDto>>>,
    alternate_titles_lock: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AlternateTitleUpdateDto {
    label: Option<String>,
    title: Option<String>,
}

/// An enum constant by its Kotlin name, or `null`.
fn named<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr<Err = String>,
{
    let name: Option<String> = Option::deserialize(deserializer)?;
    name.map(|name| name.parse().map_err(D::Error::custom))
        .transpose()
}

/// [`named`], distinguishing `null` from an absent field.
fn present_named<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr<Err = String>,
{
    named(deserializer).map(Some)
}

impl SeriesMetadataUpdateDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::null_or_not_blank("title", self.title.as_deref(), &mut violations);
        validation::null_or_not_blank("titleSort", self.title_sort.as_deref(), &mut violations);
        if let Some(Some(age_rating)) = self.age_rating {
            validation::positive_or_zero("ageRating", age_rating, &mut violations);
        }
        validation::null_or_blank_or_bcp47("language", self.language.as_deref(), &mut violations);
        if let Some(Some(count)) = self.total_book_count {
            validation::positive("totalBookCount", count.into(), &mut violations);
        }
        if let Some(Some(links)) = &self.links {
            web_link_violations(links, &mut violations);
        }
        if let Some(Some(titles)) = &self.alternate_titles {
            let pairs = titles
                .iter()
                .map(|title| [title.label.as_deref(), title.title.as_deref()]);
            not_blank_pairs(
                "alternateTitles",
                ["label", "title"],
                pairs,
                &mut violations,
            );
        }
        violations
    }

    pub(crate) fn into_update(self) -> SeriesMetadataUpdate {
        SeriesMetadataUpdate {
            status: self.status,
            status_lock: self.status_lock,
            title: self.title,
            title_lock: self.title_lock,
            title_sort: self.title_sort,
            title_sort_lock: self.title_sort_lock,
            summary: self.summary,
            summary_lock: self.summary_lock,
            reading_direction: self.reading_direction,
            reading_direction_lock: self.reading_direction_lock,
            publisher: self.publisher,
            publisher_lock: self.publisher_lock,
            age_rating: self.age_rating,
            age_rating_lock: self.age_rating_lock,
            language: self.language.map(|language| {
                if language.trim().is_empty() {
                    String::new()
                } else {
                    normalize_language_tag(&language)
                        .expect("Validation rejects an invalid language")
                }
            }),
            language_lock: self.language_lock,
            genres: self.genres.map(Option::unwrap_or_default),
            genres_lock: self.genres_lock,
            tags: self.tags.map(Option::unwrap_or_default),
            tags_lock: self.tags_lock,
            total_book_count: self.total_book_count,
            total_book_count_lock: self.total_book_count_lock,
            sharing_labels: self.sharing_labels.map(Option::unwrap_or_default),
            sharing_labels_lock: self.sharing_labels_lock,
            links: self.links.map(|links| web_links(links.unwrap_or_default())),
            links_lock: self.links_lock,
            alternate_titles: self.alternate_titles.map(|titles| {
                titles
                    .unwrap_or_default()
                    .into_iter()
                    .map(|title| AlternateTitle {
                        label: validated(title.label),
                        title: validated(title.title),
                    })
                    .collect()
            }),
            alternate_titles_lock: self.alternate_titles_lock,
        }
    }
}
