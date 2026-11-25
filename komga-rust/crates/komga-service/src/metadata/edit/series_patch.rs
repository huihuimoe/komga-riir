//! Kotlin `SeriesMetadataPatch`, its aggregation over books in `SeriesMetadataLifecycle`, and the
//! series part of `MetadataApplier`.

use komga_types::{ReadingDirection, SeriesMetadata, SeriesStatus};

use super::patch::unless_locked;

/// `None` fields were not found and leave the stored value as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub(in crate::metadata) struct SeriesMetadataPatch {
    pub title: Option<String>,
    pub title_sort: Option<String>,
    pub status: Option<SeriesStatus>,
    pub summary: Option<String>,
    pub reading_direction: Option<ReadingDirection>,
    pub publisher: Option<String>,
    pub age_rating: Option<i32>,
    pub language: Option<String>,
    pub genres: Option<Vec<String>>,
    pub total_book_count: Option<i32>,
    pub collections: Vec<String>,
}

/// `handlePatchForSeriesMetadata(patches)`: the most frequent value of each field, the highest age
/// rating and total book count, and every genre; never a summary or collections.
pub(in crate::metadata) fn aggregate(patches: &[SeriesMetadataPatch]) -> SeriesMetadataPatch {
    let mut genres: Vec<String> = Vec::new();
    for genre in patches
        .iter()
        .filter_map(|patch| patch.genres.as_ref())
        .flatten()
    {
        if !genres.contains(genre) {
            genres.push(genre.clone());
        }
    }
    SeriesMetadataPatch {
        title: most_frequent(patches.iter().map(|patch| &patch.title)),
        title_sort: most_frequent(patches.iter().map(|patch| &patch.title_sort)),
        status: most_frequent(patches.iter().map(|patch| &patch.status)),
        summary: None,
        reading_direction: most_frequent(patches.iter().map(|patch| &patch.reading_direction)),
        publisher: most_frequent(patches.iter().map(|patch| &patch.publisher)),
        age_rating: patches.iter().filter_map(|patch| patch.age_rating).max(),
        language: most_frequent(patches.iter().map(|patch| &patch.language)),
        genres: (!genres.is_empty()).then_some(genres),
        total_book_count: patches
            .iter()
            .filter_map(|patch| patch.total_book_count)
            .max(),
        collections: Vec::new(),
    }
}

/// Kotlin `mostFrequent`: among the found values, the one found most often; on a tie, the one
/// found first.
fn most_frequent<'a, T: PartialEq + Clone + 'a>(
    values: impl Iterator<Item = &'a Option<T>>,
) -> Option<T> {
    let mut counts: Vec<(&T, usize)> = Vec::new();
    for value in values.flatten() {
        match counts.iter_mut().find(|(counted, _)| *counted == value) {
            Some((_, count)) => *count += 1,
            None => counts.push((value, 1)),
        }
    }
    let mut most: Option<(&T, usize)> = None;
    for (value, count) in counts {
        if most.is_none_or(|(_, most_count)| count > most_count) {
            most = Some((value, count));
        }
    }
    most.map(|(value, _)| value.clone())
}

/// `MetadataApplier.apply`: each found value replaces the stored one unless that field is locked.
pub(in crate::metadata) fn apply(
    patch: &SeriesMetadataPatch,
    metadata: SeriesMetadata,
) -> SeriesMetadata {
    let m = metadata;
    SeriesMetadata {
        status: unless_locked(m.status, patch.status.as_ref(), m.status_lock),
        title: unless_locked(m.title, patch.title.as_ref(), m.title_lock),
        title_sort: unless_locked(m.title_sort, patch.title_sort.as_ref(), m.title_sort_lock),
        summary: unless_locked(m.summary, patch.summary.as_ref(), m.summary_lock),
        reading_direction: unless_locked(
            m.reading_direction,
            patch.reading_direction.map(Some).as_ref(),
            m.reading_direction_lock,
        ),
        age_rating: unless_locked(
            m.age_rating,
            patch.age_rating.map(Some).as_ref(),
            m.age_rating_lock,
        ),
        publisher: unless_locked(m.publisher, patch.publisher.as_ref(), m.publisher_lock),
        language: unless_locked(m.language, patch.language.as_ref(), m.language_lock),
        genres: unless_locked(m.genres, patch.genres.as_ref(), m.genres_lock),
        total_book_count: unless_locked(
            m.total_book_count,
            patch.total_book_count.map(Some).as_ref(),
            m.total_book_count_lock,
        ),
        ..m
    }
    .normalized()
}

#[cfg(test)]
mod tests {
    use komga_types::{ReadingDirection, SeriesId, SeriesMetadata, SeriesStatus};

    use super::{SeriesMetadataPatch, aggregate, apply};

    fn titled(title: &str) -> SeriesMetadataPatch {
        SeriesMetadataPatch {
            title: Some(title.to_owned()),
            ..SeriesMetadataPatch::default()
        }
    }

    fn metadata() -> SeriesMetadata {
        SeriesMetadata {
            series_id: SeriesId::from("SERIES".to_owned()),
            status: SeriesStatus::Ongoing,
            status_lock: false,
            title: "Folder".to_owned(),
            title_lock: false,
            title_sort: "Folder".to_owned(),
            title_sort_lock: false,
            summary: "Kept".to_owned(),
            summary_lock: false,
            reading_direction: None,
            reading_direction_lock: false,
            publisher: String::new(),
            publisher_lock: false,
            age_rating: Some(3),
            age_rating_lock: false,
            language: String::new(),
            language_lock: false,
            genres: vec!["old".to_owned()],
            genres_lock: false,
            total_book_count: None,
            total_book_count_lock: false,
            tags: Vec::new(),
            tags_lock: false,
            sharing_labels: Vec::new(),
            sharing_labels_lock: false,
            links: Vec::new(),
            links_lock: false,
            alternate_titles: Vec::new(),
            alternate_titles_lock: false,
        }
    }

    #[test]
    fn the_most_frequent_value_wins_and_ties_go_to_the_first_found() {
        let patches = [titled("B"), titled("A"), titled("A"), titled("B")];
        assert_eq!(aggregate(&patches).title.as_deref(), Some("B"));
        let patches = [titled("B"), titled("A"), titled("A")];
        assert_eq!(aggregate(&patches).title.as_deref(), Some("A"));
        assert_eq!(aggregate(&[]), SeriesMetadataPatch::default());
    }

    #[test]
    fn ratings_and_counts_take_the_highest_and_genres_join() {
        let patch = |age_rating, count, genres: &[&str]| SeriesMetadataPatch {
            age_rating,
            total_book_count: count,
            genres: Some(genres.iter().map(|genre| (*genre).to_owned()).collect()),
            summary: Some("Never aggregated".to_owned()),
            collections: vec!["Never aggregated".to_owned()],
            ..SeriesMetadataPatch::default()
        };
        let aggregated = aggregate(&[
            patch(Some(13), Some(5), &["Action", "Drama"]),
            patch(None, Some(12), &["Drama", "Comedy"]),
            patch(Some(18), None, &[]),
        ]);
        assert_eq!(
            aggregated,
            SeriesMetadataPatch {
                age_rating: Some(18),
                total_book_count: Some(12),
                genres: Some(vec![
                    "Action".to_owned(),
                    "Drama".to_owned(),
                    "Comedy".to_owned()
                ]),
                ..SeriesMetadataPatch::default()
            }
        );
    }

    #[test]
    fn found_values_replace_unlocked_fields_only() {
        let original = SeriesMetadata {
            title_lock: true,
            genres_lock: true,
            ..metadata()
        };
        let patch = SeriesMetadataPatch {
            title: Some("Title".to_owned()),
            title_sort: Some(" Title ".to_owned()),
            reading_direction: Some(ReadingDirection::RightToLeft),
            publisher: Some(" Publisher ".to_owned()),
            language: Some("en".to_owned()),
            genres: Some(vec!["new".to_owned()]),
            total_book_count: Some(4),
            ..SeriesMetadataPatch::default()
        };
        assert_eq!(
            apply(&patch, original.clone()),
            SeriesMetadata {
                title_sort: "Title".to_owned(),
                reading_direction: Some(ReadingDirection::RightToLeft),
                publisher: "Publisher".to_owned(),
                language: "en".to_owned(),
                total_book_count: Some(4),
                ..original
            }
        );
        assert_eq!(
            apply(&SeriesMetadataPatch::default(), metadata()),
            metadata()
        );
    }
}
