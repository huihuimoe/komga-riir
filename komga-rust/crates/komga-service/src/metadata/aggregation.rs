//! Kotlin `SeriesMetadataLifecycle.aggregateMetadata` and `MetadataAggregator`: what the
//! metadata of the books of a series have in common.

use komga_db::{
    BookDao, BookMetadataAggregationDao, BookMetadataDao, SearchIndexPendingDao, SeriesDao,
};
use komga_types::{
    BookMetadata, BookMetadataAggregation, DomainEvent, SearchEntity, SeriesId, SeriesRef,
};
use tracing::{info, warn};

use super::{MetadataError, MetadataService};

impl MetadataService {
    /// Stores the aggregation of the metadata of the series' books, soft-deleted ones included,
    /// as Kotlin does. The books are read outside the write transaction, like Kotlin's.
    pub(crate) async fn aggregate_series_metadata(
        &self,
        id: &SeriesId,
    ) -> Result<(), MetadataError> {
        let mut tx = self.db.begin().await?;
        let Some(series) = SeriesDao::find_by_id(&mut tx, id).await? else {
            tx.rollback().await?;
            warn!(series = %id, "Cannot aggregate metadata: Series does not exist");
            return Ok(());
        };
        info!(series = %id, "Aggregate book metadata for series");
        let mut metadatas = Vec::new();
        for book in BookDao::find_all_by_series(&mut tx, id).await? {
            metadatas.extend(BookMetadataDao::find_by_id(&mut tx, &book.id).await?);
        }
        tx.rollback().await?;

        let aggregation = aggregate(id, metadatas);
        let mut tx = self.db.begin_immediate().await?;
        BookMetadataAggregationDao::update(&mut tx, &aggregation).await?;
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::Series, &[id.as_str()]).await?;
        tx.record(DomainEvent::SeriesUpdated(SeriesRef::from(&series)));
        self.events.commit(tx).await?;
        self.search.notify();
        Ok(())
    }
}

/// `MetadataAggregator.aggregate`: distinct authors, notable entities, and tags in book order,
/// the earliest release date, and the first non-blank summary by number sort.
fn aggregate(series_id: &SeriesId, mut metadatas: Vec<BookMetadata>) -> BookMetadataAggregation {
    let mut authors = Vec::new();
    let mut notable_entities = Vec::new();
    let mut tags = Vec::new();
    for metadata in &metadatas {
        for author in &metadata.authors {
            if !authors.contains(author) {
                authors.push(author.clone());
            }
        }
        for entity in &metadata.notable_entities {
            if !notable_entities.contains(entity) {
                notable_entities.push(entity.clone());
            }
        }
        for tag in &metadata.tags {
            if !tags.contains(tag) {
                tags.push(tag.clone());
            }
        }
    }
    let release_date = metadatas
        .iter()
        .filter_map(|metadata| metadata.release_date)
        .min();

    // A stable sort, like Kotlin `sortedBy`, whose `Float` order `total_cmp` matches.
    metadatas.sort_by(|a, b| a.number_sort.total_cmp(&b.number_sort));
    let (summary, summary_number) = metadatas
        .into_iter()
        .find(|metadata| !metadata.summary.trim().is_empty())
        .map(|metadata| (metadata.summary, metadata.number))
        .unwrap_or_default();

    BookMetadataAggregation {
        series_id: series_id.clone(),
        authors,
        notable_entities,
        tags,
        release_date,
        summary,
        summary_number,
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use komga_types::{BookAuthor, BookId, BookMetadata, BookMetadataAggregation, NotableEntity};

    use super::aggregate;

    fn book(number: &str, number_sort: f32, summary: &str) -> BookMetadata {
        BookMetadata {
            book_id: BookId::from(format!("BOOK{number}")),
            title: String::new(),
            title_lock: false,
            summary: summary.to_owned(),
            summary_lock: false,
            number: number.to_owned(),
            number_lock: false,
            number_sort,
            number_sort_lock: false,
            release_date: None,
            release_date_lock: false,
            authors: Vec::new(),
            authors_lock: false,
            notable_entities: Vec::new(),
            notable_entities_lock: false,
            tags: Vec::new(),
            tags_lock: false,
            isbn: String::new(),
            isbn_lock: false,
            links: Vec::new(),
            links_lock: false,
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|&value| value.to_owned()).collect()
    }

    #[test]
    fn collections_are_distinct_in_book_order_and_the_earliest_date_wins() {
        let first = BookMetadata {
            authors: vec![
                BookAuthor::new("Ann", "writer"),
                BookAuthor::new("Bob", "writer"),
            ],
            notable_entities: vec![NotableEntity::new("character", "Eve")],
            tags: strings(&["b", "a"]),
            release_date: Some(date(2021, 5, 1)),
            ..book("2", 2.0, "")
        };
        let second = BookMetadata {
            authors: vec![
                BookAuthor::new("Ann", "penciller"),
                BookAuthor::new("Ann", "writer"),
            ],
            notable_entities: vec![
                NotableEntity::new("location", "Eve"),
                NotableEntity::new("character", "Eve"),
            ],
            tags: strings(&["a", "c"]),
            release_date: Some(date(2020, 1, 1)),
            ..book("1", 1.0, "")
        };
        let aggregation = aggregate(&"S".into(), vec![first, second, book("3", 3.0, "")]);
        assert_eq!(
            aggregation,
            BookMetadataAggregation {
                series_id: "S".into(),
                authors: vec![
                    BookAuthor::new("Ann", "writer"),
                    BookAuthor::new("Bob", "writer"),
                    BookAuthor::new("Ann", "penciller"),
                ],
                notable_entities: vec![
                    NotableEntity::new("character", "Eve"),
                    NotableEntity::new("location", "Eve"),
                ],
                tags: strings(&["b", "a", "c"]),
                release_date: Some(date(2020, 1, 1)),
                summary: String::new(),
                summary_number: String::new(),
            }
        );
    }

    #[test]
    fn the_summary_comes_from_the_first_book_by_number_sort_that_has_one() {
        let books = vec![
            book("10", 10.0, "Tenth"),
            book("2b", 2.0, "Second b"),
            book("1", 1.0, " \t"),
            book("2a", 2.0, "Second a"),
        ];
        let aggregation = aggregate(&"S".into(), books);
        assert_eq!(
            (
                aggregation.summary.as_str(),
                aggregation.summary_number.as_str()
            ),
            ("Second b", "2b"),
            "blank summaries are skipped and ties keep the book order"
        );
        assert_eq!(aggregation.release_date, None);
    }
}
