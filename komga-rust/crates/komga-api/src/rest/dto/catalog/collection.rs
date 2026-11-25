//! Collection requests and representation.

use komga_service::{CollectionUpdate, NewCollection};
use komga_types::{CollectionId, SeriesCollection, SeriesId};
use serde::{Deserialize, Serialize};

use crate::error::Violation;
use crate::rest::validation;

/// `@JsonFormat(pattern = "yyyy-MM-dd'T'HH:mm:ss'Z'")`.
const DATE_TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDto {
    pub id: CollectionId,
    pub name: String,
    pub ordered: bool,
    pub series_ids: Vec<SeriesId>,
    pub created_date: String,
    pub last_modified_date: String,
    pub filtered: bool,
}

impl From<SeriesCollection> for CollectionDto {
    fn from(collection: SeriesCollection) -> Self {
        Self {
            id: collection.id,
            name: collection.name,
            ordered: collection.ordered,
            series_ids: collection.series_ids,
            created_date: collection
                .created_date
                .strftime(DATE_TIME_FORMAT)
                .to_string(),
            last_modified_date: collection
                .last_modified_date
                .strftime(DATE_TIME_FORMAT)
                .to_string(),
            filtered: collection.filtered,
        }
    }
}

/// `ordered` is a Kotlin `Boolean` without a default, which Jackson reads as `false` when
/// absent.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionCreationDto {
    pub name: String,
    #[serde(default)]
    pub ordered: bool,
    pub series_ids: Vec<SeriesId>,
}

impl CollectionCreationDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::not_blank("name", &self.name, &mut violations);
        validation::not_empty("seriesIds", &self.series_ids, &mut violations);
        validation::unique_elements("seriesIds", Some(&self.series_ids), &mut violations);
        violations
    }

    pub(crate) fn into_new(self) -> NewCollection {
        NewCollection {
            name: self.name,
            ordered: self.ordered,
            series_ids: self.series_ids,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionUpdateDto {
    pub name: Option<String>,
    pub ordered: Option<bool>,
    pub series_ids: Option<Vec<SeriesId>>,
}

impl CollectionUpdateDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::null_or_not_blank("name", self.name.as_deref(), &mut violations);
        validation::null_or_not_empty("seriesIds", self.series_ids.as_deref(), &mut violations);
        validation::unique_elements("seriesIds", self.series_ids.as_deref(), &mut violations);
        violations
    }

    pub(crate) fn into_update(self) -> CollectionUpdate {
        CollectionUpdate {
            name: self.name,
            ordered: self.ordered,
            series_ids: self.series_ids,
        }
    }
}
