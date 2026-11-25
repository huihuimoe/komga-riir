//! The documents of the indexed entities, as the database holds them.

use std::collections::HashMap;

use jiff::civil::Date;
use komga_db::{
    BookDao, BookSearchFields, DbError, DbTransaction, PendingUpdate, ReadListDao,
    SeriesCollectionDao, SeriesDao, SeriesSearchFields,
};
use komga_search::{IndexUpdate, SearchDocument};
use komga_types::SearchEntity;

/// The documents of the `pending` entities as `transaction` sees them; a missing one is deleted.
pub(super) async fn current_documents(
    transaction: &mut DbTransaction,
    entity: SearchEntity,
    pending: &[PendingUpdate],
) -> Result<Vec<IndexUpdate>, DbError> {
    let ids: Vec<&str> = pending.iter().map(|update| update.id.as_str()).collect();
    let documents: Vec<SearchDocument> = match entity {
        SearchEntity::Book => BookDao::find_search_fields(transaction, &ids)
            .await?
            .into_iter()
            .map(book_document)
            .collect(),
        SearchEntity::Series => SeriesDao::find_search_fields(transaction, &ids)
            .await?
            .into_iter()
            .map(series_document)
            .collect(),
        SearchEntity::Collection => name_documents(
            entity,
            SeriesCollectionDao::find_names(transaction, &ids).await?,
        ),
        SearchEntity::ReadList => {
            name_documents(entity, ReadListDao::find_names(transaction, &ids).await?)
        }
    };
    let mut documents: HashMap<String, SearchDocument> = documents
        .into_iter()
        .map(|document| (document.id().to_owned(), document))
        .collect();
    let updates = pending
        .iter()
        .map(|update| match documents.remove(&update.id) {
            Some(document) => IndexUpdate::Upsert(document),
            None => IndexUpdate::Delete {
                entity,
                id: update.id.clone(),
            },
        })
        .collect();
    Ok(updates)
}

fn name_documents(entity: SearchEntity, names: Vec<(String, String)>) -> Vec<SearchDocument> {
    names
        .into_iter()
        .map(|(id, name)| {
            let mut document = SearchDocument::new(entity, id);
            document.add("name", &name);
            document
        })
        .collect()
}

/// `BookDto.toDocument`, merged with `SeriesDto.oneshotDocument` for a oneshot.
fn book_document(book: BookSearchFields) -> SearchDocument {
    let mut document = SearchDocument::new(SearchEntity::Book, book.id);
    document.add("title", &book.title);
    document.add("isbn", &book.isbn);
    for tag in &book.tags {
        document.add("tag", tag);
    }
    for author in &book.authors {
        document.add("author", &author.name);
        document.add(&author.role, &author.name);
    }
    if let Some(date) = book.release_date {
        document.add("release_date", &year(date));
    }
    document.add("status", &book.media_status);
    document.add("deleted", &book.deleted.to_string());
    document.add("oneshot", &book.oneshot.is_some().to_string());
    if let Some(series) = &book.oneshot {
        document.add("publisher", &series.publisher);
        document.add("status", &series.status);
        document.add("reading_direction", &series.reading_direction);
        if let Some(age_rating) = series.age_rating {
            document.add("age_rating", &age_rating.to_string());
        }
        if !series.language.trim().is_empty() {
            document.add("language", &series.language);
        }
        for genre in &series.genres {
            document.add("genre", genre);
        }
        for label in &series.sharing_labels {
            document.add("sharing_label", label);
        }
        document.add("complete", "true");
    }
    document
}

/// `SeriesDto.toDocument`.
fn series_document(series: SeriesSearchFields) -> SearchDocument {
    let mut document = SearchDocument::new(SearchEntity::Series, series.id);
    document.add("title", &series.title);
    if series.title_sort != series.title {
        document.add("title", &series.title_sort);
    }
    for title in &series.alternate_titles {
        document.add("title", title);
    }
    document.add("publisher", &series.publisher);
    document.add("status", &series.status);
    document.add("reading_direction", &series.reading_direction);
    if let Some(age_rating) = series.age_rating {
        document.add("age_rating", &age_rating.to_string());
    }
    if !series.language.trim().is_empty() {
        document.add("language", &series.language);
    }
    for (field, tags) in [
        ("series_tag", &series.tags),
        ("book_tag", &series.book_tags),
    ] {
        for tag in tags {
            document.add(field, tag);
            document.add("tag", tag);
        }
    }
    for genre in &series.genres {
        document.add("genre", genre);
    }
    for label in &series.sharing_labels {
        document.add("sharing_label", label);
    }
    if let Some(total) = series.total_book_count {
        document.add("total_book_count", &total.to_string());
    }
    document.add("book_count", &series.book_count.to_string());
    for author in &series.book_authors {
        document.add("author", &author.name);
        document.add(&author.role, &author.name);
    }
    if let Some(date) = series.book_release_date {
        document.add("release_date", &year(date));
    }
    document.add("deleted", &series.deleted.to_string());
    document.add("oneshot", &series.oneshot.to_string());
    if let Some(total) = series.total_book_count {
        document.add("complete", &(total == series.book_count).to_string());
    }
    document
}

/// Lucene `DateTools.dateToString` at `YEAR` resolution of the date at UTC midnight.
fn year(date: Date) -> String {
    format!("{:04}", date.year())
}
