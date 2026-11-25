//! Kotlin `MylarSeriesProvider`: series metadata from the Mylar `series.json` of a series
//! directory.

use std::path::Path;

use komga_types::SeriesStatus;
use serde::Deserialize;
use serde_json::Value;
use tokio::fs;
use tracing::error;

use super::super::edit::series_patch::SeriesMetadataPatch;

pub(super) const SERIES_JSON: &str = "series.json";

/// Kotlin `dto.Series`; unknown properties are ignored.
#[derive(Debug, Deserialize)]
struct SeriesJson {
    metadata: MylarMetadata,
}

/// Kotlin `dto.MylarMetadata`: the underscored fields are unused but required, as their
/// non-null Kotlin types make them; `imprint` is not read.
#[derive(Debug, Deserialize)]
struct MylarMetadata {
    #[serde(rename = "type")]
    _type: String,
    publisher: String,
    name: String,
    /// A string or, as Mylar writes it, a number.
    #[serde(rename = "comicid", alias = "cid")]
    comic_id: Value,
    year: i32,
    description_text: Option<String>,
    description_formatted: Option<String>,
    volume: Option<i32>,
    #[serde(rename = "booktype")]
    _book_type: String,
    age_rating: Option<AgeRating>,
    #[serde(rename = "comic_image", alias = "ComicImage")]
    _comic_image: String,
    total_issues: i32,
    #[serde(rename = "publication_run")]
    _publication_run: String,
    status: Status,
}

#[derive(Debug, Deserialize)]
enum AgeRating {
    All,
    #[serde(rename = "9+")]
    Nine,
    #[serde(rename = "12+")]
    Twelve,
    #[serde(rename = "15+")]
    Fifteen,
    #[serde(rename = "17+")]
    Seventeen,
    Adult,
}

#[derive(Debug, Deserialize)]
enum Status {
    Ended,
    Continuing,
}

/// `getSeriesMetadata` for the `series.json` at `path`; `None`, logged, when it cannot be read
/// or does not hold valid Mylar metadata.
pub(super) async fn series_patch(path: &Path) -> Option<SeriesMetadataPatch> {
    let parsed = match fs::read(path).await {
        Ok(content) => parse(&content).map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    parsed
        .inspect_err(|message| {
            error!(path = %path.display(), "Error while retrieving metadata from {SERIES_JSON}: {message}");
        })
        .ok()
}

fn parse(content: &[u8]) -> serde_json::Result<SeriesMetadataPatch> {
    let content = content.strip_prefix(b"\xef\xbb\xbf").unwrap_or(content);
    let metadata = serde_json::from_slice::<SeriesJson>(content)?.metadata;
    if !matches!(metadata.comic_id, Value::String(_) | Value::Number(_)) {
        return Err(serde::de::Error::custom(
            "comicid is not a string or a number",
        ));
    }
    Ok(patch(metadata))
}

fn patch(metadata: MylarMetadata) -> SeriesMetadataPatch {
    // Mylar names a series' later volumes by their start year.
    let title = match metadata.volume {
        None | Some(1) => metadata.name,
        Some(_) => format!("{} ({})", metadata.name, metadata.year),
    };
    SeriesMetadataPatch {
        title_sort: Some(title.clone()),
        title: Some(title),
        status: Some(match metadata.status {
            Status::Ended => SeriesStatus::Ended,
            Status::Continuing => SeriesStatus::Ongoing,
        }),
        summary: metadata.description_formatted.or(metadata.description_text),
        publisher: Some(metadata.publisher),
        age_rating: metadata.age_rating.map(|rating| match rating {
            AgeRating::All => 0,
            AgeRating::Nine => 9,
            AgeRating::Twelve => 12,
            AgeRating::Fifteen => 15,
            AgeRating::Seventeen => 17,
            AgeRating::Adult => 18,
        }),
        total_book_count: Some(metadata.total_issues),
        ..SeriesMetadataPatch::default()
    }
}

#[cfg(test)]
mod tests {
    //! Documents and expectations of Kotlin `SeriesTest` and `MylarSeriesProviderTest`.

    use komga_types::SeriesStatus;
    use serde_json::{Value, json};

    use super::parse;
    use crate::metadata::edit::series_patch::SeriesMetadataPatch;

    /// The first `SeriesTest` document with `changes` merged into its metadata.
    fn document(changes: &Value) -> Vec<u8> {
        let mut document = json!({
            "version": "1.0.1",
            "metadata": {
                "type": "comicSeries",
                "publisher": "DC Comics",
                "imprint": null,
                "name": "American Vampire 1976",
                "cid": 130_865,
                "year": 2020,
                "description_text": "Text",
                "description_formatted": "Formatted",
                "volume": null,
                "booktype": "Print",
                "age_rating": "Adult",
                "collects": null,
                "ComicImage": "https://comicvine.gamespot.com/a/uploads/scale_large/6/67663/7603293-01.jpg",
                "total_issues": 9,
                "publication_run": "December 2020 - Present",
                "status": "Continuing"
            }
        });
        let metadata = document["metadata"]
            .as_object_mut()
            .expect("Metadata object");
        for (key, value) in changes.as_object().expect("Changes object") {
            if value == "<missing>" {
                metadata.remove(key);
            } else {
                metadata.insert(key.clone(), value.clone());
            }
        }
        serde_json::to_vec(&document).expect("JSON")
    }

    #[test]
    fn series_json_gives_the_series_metadata() {
        let patch = parse(&document(&json!({}))).expect("Valid document");
        assert_eq!(
            patch,
            SeriesMetadataPatch {
                title: Some("American Vampire 1976".to_owned()),
                title_sort: Some("American Vampire 1976".to_owned()),
                status: Some(SeriesStatus::Ongoing),
                summary: Some("Formatted".to_owned()),
                publisher: Some("DC Comics".to_owned()),
                age_rating: Some(18),
                total_book_count: Some(9),
                ..SeriesMetadataPatch::default()
            }
        );
    }

    #[test]
    fn later_volumes_are_titled_by_year_and_optional_values_fall_back() {
        let changes = json!({
            "cid": "<missing>",
            "comicid": "119731",
            "comic_image": "image",
            "ComicImage": "<missing>",
            "volume": 4,
            "description_formatted": null,
            "age_rating": null,
            "status": "Ended",
        });
        let patch = parse(&document(&changes)).expect("Valid document");
        assert_eq!(patch.title.as_deref(), Some("American Vampire 1976 (2020)"));
        assert_eq!(patch.title_sort, patch.title);
        assert_eq!(patch.summary.as_deref(), Some("Text"));
        assert_eq!(
            (patch.age_rating, patch.status),
            (None, Some(SeriesStatus::Ended))
        );
        let first = parse(&document(&json!({"volume": 1}))).expect("Valid document");
        assert_eq!(first.title.as_deref(), Some("American Vampire 1976"));
    }

    #[test]
    fn incomplete_or_unknown_values_are_no_metadata() {
        for changes in [
            json!({"year": "<missing>"}),
            json!({"publisher": "<missing>"}),
            json!({"status": "<missing>"}),
            json!({"cid": null}),
            json!({"status": "Cancelled"}),
            json!({"age_rating": "18+"}),
        ] {
            assert!(parse(&document(&changes)).is_err(), "{changes}");
        }
        assert!(parse(b"").is_err(), "an empty file");
    }
}
